use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use egui::{
    Button, CentralPanel, Color32, Context, Frame, Id, Margin, Response, RichText, Stroke,
    TextEdit, Ui, Vec2, ViewportId,
};

const ANIMATION_REFRESH_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// Styled button variants
#[allow(dead_code)]
pub(crate) enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// A styled button that follows Dash design guidelines
pub(crate) struct StyledButton {
    text: String,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    min_width: Option<f32>,
}

#[allow(dead_code)]
pub(crate) enum ButtonSize {
    Small,
    Medium,
    Large,
}

impl StyledButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            variant: ButtonVariant::Primary,
            size: ButtonSize::Medium,
            enabled: true,
            min_width: None,
        }
    }

    pub fn primary(text: impl Into<String>) -> Self {
        Self::new(text)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let (text_color, bg_color, _hover_color, stroke) = match self.variant {
            ButtonVariant::Primary => (
                DashColors::WHITE,
                DashColors::DASH_BLUE,
                DashColors::DEEP_BLUE,
                None,
            ),
            ButtonVariant::Secondary => (
                DashColors::DASH_BLUE,
                DashColors::WHITE,
                DashColors::BACKGROUND,
                Some(Stroke::new(1.0, DashColors::DASH_BLUE)),
            ),
            ButtonVariant::Danger => (
                DashColors::WHITE,
                DashColors::ERROR,
                Color32::from_rgb(200, 0, 0),
                None,
            ),
            ButtonVariant::Ghost => (
                DashColors::TEXT_PRIMARY,
                Color32::TRANSPARENT,
                DashColors::glass_white(),
                None,
            ),
        };

        let _padding = match self.size {
            ButtonSize::Small => Vec2::new(12.0, 6.0),
            ButtonSize::Medium => Vec2::new(16.0, 8.0),
            ButtonSize::Large => Vec2::new(20.0, 10.0),
        };

        let font_size = match self.size {
            ButtonSize::Small => Typography::SCALE_SM,
            ButtonSize::Medium => Typography::SCALE_BASE,
            ButtonSize::Large => Typography::SCALE_LG,
        };

        let mut button = Button::new(RichText::new(self.text).size(font_size).color(text_color))
            .fill(if self.enabled {
                bg_color
            } else {
                DashColors::DISABLED
            })
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD));

        if let Some(stroke) = stroke {
            button = button.stroke(stroke);
        }

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
    title: Option<String>,
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
            title: None,
            padding: Spacing::CARD_PADDING,
            show_border: true,
        }
    }

    // pub fn title(mut self, title: impl Into<String>) -> Self {
    //     self.title = Some(title.into());
    //     self
    // }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    // pub fn show_border(mut self, show: bool) -> Self {
    //     self.show_border = show;
    //     self
    // }

    pub fn show<R>(self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        let stroke = if self.show_border {
            Stroke::new(1.0, DashColors::BORDER)
        } else {
            Stroke::NONE
        };

        egui::Frame::new()
            .fill(DashColors::SURFACE)
            .stroke(stroke)
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .inner_margin(egui::Margin::same(self.padding as i8))
            .shadow(Shadow::medium())
            .show(ui, |ui| {
                if let Some(title) = self.title {
                    ui.label(
                        RichText::new(title)
                            .font(Typography::heading_small())
                            .color(DashColors::TEXT_PRIMARY),
                    );
                    ui.add_space(Spacing::MD);
                }
                content(ui)
            })
            .inner
    }
}

/// Styled checkbox with Dash theme
pub(crate) struct StyledCheckbox<'a> {
    checked: &'a mut bool,
    text: String,
}

// #[allow(dead_code)]
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
    glow: bool,
}

impl GradientButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            min_width: None,
            glow: false,
        }
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    pub fn glow(mut self) -> Self {
        self.glow = true;
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
        repaint_animation(ui.ctx(), response.id);

        response
    }
}

/// Helper function to style a TextEdit with consistent theme
pub fn styled_text_edit_singleline(text: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(text).background_color(DashColors::INPUT_BACKGROUND)
}

/// Helper function to style a multiline TextEdit with consistent theme
// #[allow(dead_code)]
pub fn styled_text_edit_multiline(text: &mut String) -> TextEdit<'_> {
    TextEdit::multiline(text).background_color(DashColors::INPUT_BACKGROUND)
}

/// Helper function to create an island-style central panel
pub fn island_central_panel<R>(ctx: &Context, content: impl FnOnce(&mut Ui) -> R) -> R {
    CentralPanel::default()
        .frame(
            Frame::new()
                .fill(DashColors::BACKGROUND) // Light background instead of transparent
                .inner_margin(Margin::symmetric(20, 10)), // Increased horizontal margin to prevent edge touching
        )
        .show(ctx, |ui| {
            // Calculate responsive margins based on available width, but ensure minimum spacing
            let available_width = ui.available_width();
            let inner_margin = if available_width > 1200.0 {
                24.0 // Spacing::LG for larger screens
            } else {
                20.0 // Minimum 20px to prevent edge touching
            };

            // Create an island panel with rounded edges
            Frame::new()
                .fill(DashColors::SURFACE)
                .stroke(Stroke::new(1.0, DashColors::BORDER_LIGHT))
                .inner_margin(Margin::same(inner_margin as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| content(ui))
                .inner
        })
        .inner
}

/// Repaint animated object
fn repaint_animation(ctx: &Context, id: Id) {
    // Request repaint for animations
    ctx.request_repaint_after_for(ANIMATION_REFRESH_TIME, ViewportId(id));
}
