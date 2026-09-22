use std::sync::Arc;

use crate::{
    context::AppContext,
    ui::theme::{DashColors, Shadow, Shape, Spacing, Typography},
};
use egui::{Button, CentralPanel, Frame, Margin, Response, RichText, Stroke, TextEdit, Ui, Vec2};

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

    pub fn primary(text: impl Into<String>) -> Self {
        Self::new(text)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let mut button = Button::new(
            RichText::new(self.text)
                .size(Typography::SCALE_BASE)
                .color(DashColors::WHITE),
        )
        .fill(if self.enabled {
            DashColors::DASH_BLUE
        } else {
            DashColors::DISABLED
        })
        .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD));

        if let Some(min_width) = self.min_width {
            button = button.min_size(Vec2::new(min_width, 0.0));
        }

        let response = ui.add_enabled(self.enabled, button);

        if response.hovered() && self.enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        response
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
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .inner_margin(egui::Margin::same(self.padding as i8))
            .shadow(Shadow::medium())
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

/// Helper function to style a TextEdit with consistent theme
pub fn styled_text_edit_singleline(text: &mut String, dark_mode: bool) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .text_color(DashColors::text_primary(dark_mode))
        .background_color(DashColors::input_background(dark_mode))
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
                .fill(DashColors::surface(dark_mode))
                .stroke(Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(inner_margin as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    super::MessageBanner::show_global(ui);
                    content(ui)
                })
                .inner
        })
        .inner
}
