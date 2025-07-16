use std::sync::Arc;

use crate::{
    context::AppContext,
    ui::theme::{DashColors, Shadow, Shape, Spacing, Typography},
};
use egui::{
    Button, CentralPanel, Color32, Context, Frame, Margin, Response, RichText, Stroke, TextEdit,
    Ui, Vec2,
};

// Re-export commonly used components
pub use super::clickable_collapsing_header::ClickableCollapsingHeader;

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
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        let (text_color, bg_color, _hover_color, stroke) = match self.variant {
            ButtonVariant::Primary => (
                DashColors::WHITE,
                DashColors::DASH_BLUE,
                DashColors::DEEP_BLUE,
                None,
            ),
            ButtonVariant::Secondary => (
                DashColors::DASH_BLUE,
                if dark_mode {
                    DashColors::surface(dark_mode)
                } else {
                    DashColors::WHITE
                },
                DashColors::background(dark_mode),
                Some(Stroke::new(1.0, DashColors::DASH_BLUE)),
            ),
            ButtonVariant::Danger => (
                DashColors::WHITE,
                DashColors::ERROR,
                Color32::from_rgb(200, 0, 0),
                None,
            ),
            ButtonVariant::Ghost => (
                DashColors::text_primary(dark_mode),
                Color32::TRANSPARENT,
                DashColors::glass_white(dark_mode),
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
        let dark_mode = ui.ctx().style().visuals.dark_mode;

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
            .show(ui, |ui| {
                if let Some(title) = self.title {
                    ui.label(
                        RichText::new(title)
                            .font(Typography::heading_small())
                            .color(DashColors::text_primary(dark_mode)),
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
    app_context: Arc<AppContext>,
}

impl GradientButton {
    pub fn new(text: impl Into<String>, app_context: &Arc<AppContext>) -> Self {
        Self {
            text: text.into(),
            min_width: None,
            glow: false,
            app_context: Arc::clone(app_context),
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
        self.app_context.repaint_animation(ui.ctx());

        response
    }
}

/// Glass-morphism styled card
pub struct GlassCard {
    title: Option<String>,
    padding: f32,
}

#[allow(dead_code)]
impl GlassCard {
    pub fn new() -> Self {
        Self {
            title: None,
            padding: Spacing::CARD_PADDING,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn show<R>(self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        egui::Frame::new()
            .fill(DashColors::glass_white(dark_mode))
            .stroke(Stroke::new(1.0, DashColors::glass_border(dark_mode)))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_XL))
            .inner_margin(egui::Margin::same(self.padding as i8))
            .shadow(Shadow::medium())
            .show(ui, |ui| {
                if let Some(title) = self.title {
                    ui.label(
                        RichText::new(title)
                            .font(Typography::heading_medium())
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(Spacing::MD);
                }
                content(ui)
            })
            .inner
    }
}

impl Default for GlassCard {
    fn default() -> Self {
        Self::new()
    }
}
/// Hero section with gradient background
pub struct HeroSection {
    title: String,
    subtitle: Option<String>,
}

#[allow(dead_code)]
impl HeroSection {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn show(self, ui: &mut Ui) {
        let time = ui.ctx().input(|i| i.time as f32);
        let gradient_color = DashColors::gradient_animated(time);

        egui::Frame::new()
            .fill(gradient_color.linear_multiply(0.1))
            .stroke(Stroke::new(2.0, gradient_color))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_XL))
            .inner_margin(egui::Margin::same(Spacing::XL as i8))
            .shadow(Shadow::glow())
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new(self.title)
                            .font(Typography::heading_large())
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    if let Some(subtitle) = self.subtitle {
                        ui.add_space(Spacing::SM);
                        ui.label(
                            RichText::new(subtitle)
                                .font(Typography::body_large())
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                });
            });

        // Request repaint for animation
        ui.ctx().request_repaint();
    }
}

/// Icon with animation support
pub struct AnimatedIcon {
    icon: String,
    size: f32,
    color: Color32,
    rotation: f32,
    pulse: bool,
}

#[allow(dead_code)]
impl AnimatedIcon {
    pub fn new(icon: impl Into<String>) -> Self {
        Self {
            icon: icon.into(),
            size: Typography::SCALE_XL,
            color: DashColors::DASH_BLUE,
            rotation: 0.0,
            pulse: false,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    pub fn rotation(mut self, rotation: f32) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn pulse(mut self) -> Self {
        self.pulse = true;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let time = ui.ctx().input(|i| i.time as f32);

        let mut size = self.size;
        if self.pulse {
            let pulse_scale = 1.0 + 0.1 * (time * 2.0).sin();
            size *= pulse_scale;
        }

        let response = ui.label(RichText::new(self.icon).size(size).color(self.color));

        if self.rotation != 0.0 {
            // Apply rotation animation
            let _angle = self.rotation * time;
            // Note: egui doesn't have direct rotation support for text,
            // so this is a placeholder for future enhancement
        }

        // Request repaint for animation
        if self.pulse || self.rotation != 0.0 {
            ui.ctx().request_repaint();
        }

        response
    }
}

/// Animated gradient card
pub struct AnimatedGradientCard {
    title: Option<String>,
    padding: f32,
    gradient_index: usize,
}

#[allow(dead_code)]
impl AnimatedGradientCard {
    pub fn new() -> Self {
        Self {
            title: None,
            padding: Spacing::CARD_PADDING,
            gradient_index: 0,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn gradient_index(mut self, index: usize) -> Self {
        self.gradient_index = index;
        self
    }

    pub fn show<R>(self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        let time = ui.ctx().input(|i| i.time as f32);
        let animated_color = DashColors::gradient_animated(time);
        let pastel_color = DashColors::pastel_gradient(self.gradient_index);

        egui::Frame::new()
            .fill(pastel_color)
            .stroke(Stroke::new(2.0, animated_color))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_XL))
            .inner_margin(egui::Margin::same(self.padding as i8))
            .shadow(Shadow::elevated())
            .show(ui, |ui| {
                if let Some(title) = self.title {
                    ui.label(
                        RichText::new(title)
                            .font(Typography::heading_small())
                            .color(DashColors::TEXT_PRIMARY),
                    );
                    ui.add_space(Spacing::MD);
                }

                // Request repaint for animation
                ui.ctx().request_repaint();

                content(ui)
            })
            .inner
    }
}

impl Default for AnimatedGradientCard {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to style a TextEdit with consistent theme
pub fn styled_text_edit_singleline(text: &mut String, dark_mode: bool) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .text_color(DashColors::text_primary(dark_mode))
        .background_color(DashColors::input_background(dark_mode))
}

/// Helper function to style a multiline TextEdit with consistent theme
#[allow(dead_code)]
pub fn styled_text_edit_multiline(text: &mut String, dark_mode: bool) -> TextEdit<'_> {
    TextEdit::multiline(text)
        .text_color(DashColors::text_primary(dark_mode))
        .background_color(DashColors::input_background(dark_mode))
}

/// Helper function to create an island-style central panel
pub fn island_central_panel<R>(ctx: &Context, content: impl FnOnce(&mut Ui) -> R) -> R {
    let dark_mode = ctx.style().visuals.dark_mode;

    CentralPanel::default()
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)), // Standard margins for all panels
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
                .fill(DashColors::surface(dark_mode))
                .stroke(Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(inner_margin as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| content(ui))
                .inner
        })
        .inner
}
