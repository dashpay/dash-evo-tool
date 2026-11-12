use std::sync::Arc;

use crate::{
    context::AppContext,
    ui::theme::{ComponentStyles, DashColors, Shape, Spacing, Typography},
};
use egui::{
    Button, CentralPanel, Frame, Margin, Response, RichText, Stroke, TextBuffer, TextEdit, Ui, Vec2,
};

// Re-export commonly used components
pub use super::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
pub use super::selection_dialog::{SelectionDialog, SelectionStatus};

/// A styled primary button that follows Dash design guidelines.
pub(crate) struct StyledButton {
    text: String,
    enabled: bool,
    min_width: Option<f32>,
}

impl StyledButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            enabled: true,
            min_width: None,
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        ui.scope(|ui| {
            if let Some(width) = self.min_width {
                ui.set_min_width(width);
            }
            ComponentStyles::add_primary_button_enabled(ui, self.enabled, self.text)
        })
        .inner
    }
}

/// Styled card component
pub(crate) struct StyledCard {
    padding: f32,
    show_border: bool,
}

impl Default for StyledCard {
    fn default() -> Self {
        Self::new()
    }
}

impl StyledCard {
    pub fn new() -> Self {
        Self {
            padding: Spacing::CARD_PADDING,
            show_border: true,
        }
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn show<R>(self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        let dark_mode = ui.style().visuals.dark_mode;

        let stroke = if self.show_border {
            Stroke::new(1.0, DashColors::border(dark_mode))
        } else {
            Stroke::NONE
        };

        egui::Frame::new()
            .fill(DashColors::surface(dark_mode))
            .stroke(stroke)
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
            .inner_margin(egui::Margin::same(
                if self.padding == Spacing::CARD_PADDING
                    && ui.ctx().content_rect().width() <= 1100.0
                {
                    Spacing::CARD_PADDING_COMPACT as i8
                } else {
                    self.padding as i8
                },
            ))
            .show(ui, content)
            .inner
    }
}

/// Styled checkbox with Dash theme
pub(crate) struct StyledCheckbox<'a> {
    checked: &'a mut bool,
    text: String,
}

impl<'a> StyledCheckbox<'a> {
    pub fn new(checked: &'a mut bool, text: impl Into<String>) -> Self {
        Self {
            checked,
            text: text.into(),
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let checkbox = egui::Checkbox::new(self.checked, self.text);

        // Apply custom styling
        let response = ui.add(checkbox);

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        response
    }
}

/// Gradient button with animated effects
pub(crate) struct GradientButton {
    text: String,
    min_width: Option<f32>,
    app_context: Arc<AppContext>,
}

impl GradientButton {
    pub fn new(text: impl Into<String>, app_context: &Arc<AppContext>) -> Self {
        Self {
            text: text.into(),
            min_width: None,
            app_context: Arc::clone(app_context),
        }
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let time = ui.ctx().input(|i| i.time as f32);
        let animated_color = DashColors::gradient_animated(time);

        let mut button = Button::new(
            RichText::new(self.text)
                .color(DashColors::WHITE)
                .size(Typography::SCALE_BASE),
        )
        .fill(animated_color)
        .stroke(Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD));

        if let Some(width) = self.min_width {
            button = button.min_size(Vec2::new(width, 36.0));
        }

        let response = ui.add(button);

        // Request repaint for animation
        self.app_context.repaint_animation(ui.ctx());

        response
    }
}

/// Single-line field with the shared form padding and minimum target height.
pub fn styled_text_edit_singleline(text: &mut dyn TextBuffer) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .margin(Spacing::INPUT_MARGIN)
        .min_size(Vec2::new(0.0, Spacing::INPUT_HEIGHT))
}

/// Multiline field with the same typography and padding as single-line fields.
pub fn styled_text_edit_multiline(text: &mut dyn TextBuffer) -> TextEdit<'_> {
    TextEdit::multiline(text)
        .margin(Spacing::INPUT_MARGIN)
        .min_size(Vec2::new(0.0, Spacing::INPUT_HEIGHT))
}

/// Helper function to create an island-style central panel.
pub fn island_central_panel<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;

    CentralPanel::default()
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)), // Standard margins for all panels
        )
        .show(ui, |ui| {
            // Calculate responsive margins based on available width, but ensure minimum spacing
            let available_width = ui.available_width();
            let inner_margin = if available_width > 1200.0 {
                24.0 // Spacing::LG for larger screens
            } else {
                20.0 // Minimum 20px to prevent edge touching
            };

            // Create an island panel with rounded edges
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::same(inner_margin as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .show(ui, |ui| {
                    super::MessageBanner::show_global(ui);
                    content(ui)
                })
                .inner
        })
        .inner
}
