use crate::ui::theme::{DashColors, MessageType, Shadow, Shape, Spacing, Typography};
use egui::{
    Button, CentralPanel, Color32, Context, Frame, Id, Margin, Response, RichText, Stroke,
    TextEdit, Ui, Vec2, ViewportId,
};

const ANIMATION_REFRESH_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// Styled button variants
#[allow(dead_code)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// A styled button that follows Dash design guidelines
pub struct StyledButton {
    text: String,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    min_width: Option<f32>,
}

#[allow(dead_code)]
pub enum ButtonSize {
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

    // Unused methods commented out to eliminate warnings
    // pub fn secondary(text: impl Into<String>) -> Self {
    //     Self::new(text).variant(ButtonVariant::Secondary)
    // }

    // pub fn danger(text: impl Into<String>) -> Self {
    //     Self::new(text).variant(ButtonVariant::Danger)
    // }

    // pub fn ghost(text: impl Into<String>) -> Self {
    //     Self::new(text).variant(ButtonVariant::Ghost)
    // }

    // pub fn size(mut self, size: ButtonSize) -> Self {
    //     self.size = size;
    //     self
    // }

    // pub fn enabled(mut self, enabled: bool) -> Self {
    //     self.enabled = enabled;
    //     self
    // }

    // pub fn min_width(mut self, width: f32) -> Self {
    //     self.min_width = Some(width);
    //     self
    // }

    // pub fn variant(mut self, variant: ButtonVariant) -> Self {
    //     self.variant = variant;
    //     self
    // }

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
pub struct StyledCard {
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

// Styled text input with Dash theme - commented out as it's not currently used
// #[allow(dead_code)]
// pub struct StyledTextInput {
//     hint: Option<String>,
//     multiline: bool,
//     desired_width: Option<f32>,
//     desired_rows: Option<usize>,
// }
//
// impl StyledTextInput {
//     pub fn new() -> Self {
//         Self {
//             hint: None,
//             multiline: false,
//             desired_width: None,
//             desired_rows: None,
//         }
//     }
//
//     pub fn hint(mut self, hint: impl Into<String>) -> Self {
//         self.hint = Some(hint.into());
//         self
//     }
//
//     pub fn multiline(mut self) -> Self {
//         self.multiline = true;
//         self
//     }
//
//     pub fn desired_width(mut self, width: f32) -> Self {
//         self.desired_width = Some(width);
//         self
//     }
//
//     pub fn desired_rows(mut self, rows: usize) -> Self {
//         self.desired_rows = Some(rows);
//         self
//     }
//
//     pub fn show(self, ui: &mut Ui, text: &mut String) -> Response {
//         let mut text_edit = if self.multiline {
//             egui::TextEdit::multiline(text)
//         } else {
//             egui::TextEdit::singleline(text)
//         };
//
//         // Explicitly set the background color to INPUT_BACKGROUND
//         text_edit = text_edit.background_color(DashColors::INPUT_BACKGROUND);
//
//         if let Some(hint) = self.hint {
//             text_edit = text_edit.hint_text(hint);
//         }
//
//         if let Some(width) = self.desired_width {
//             text_edit = text_edit.desired_width(width);
//         }
//
//         if let Some(rows) = self.desired_rows {
//             text_edit = text_edit.desired_rows(rows);
//         }
//
//         ui.add(text_edit)
//     }
// }

/// Styled message component for notifications
pub struct StyledMessage {
    text: String,
    message_type: MessageType,
    show_icon: bool,
}

#[allow(dead_code)]
impl StyledMessage {
    pub fn new(text: impl Into<String>, message_type: MessageType) -> Self {
        Self {
            text: text.into(),
            message_type,
            show_icon: true,
        }
    }

    pub fn show_icon(mut self, show: bool) -> Self {
        self.show_icon = show;
        self
    }

    pub fn show(self, ui: &mut Ui) {
        let color = self.message_type.color();
        let bg_color = self.message_type.background_color();

        egui::Frame::new()
            .fill(bg_color)
            .stroke(Stroke::new(1.0, color))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
            .inner_margin(egui::Margin::same(Spacing::SM_I8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.show_icon {
                        let icon = match self.message_type {
                            MessageType::Success => "✓",
                            MessageType::Error => "✗",
                            MessageType::Warning => "!",
                            MessageType::Info => "i",
                        };
                        ui.label(RichText::new(icon).color(color).strong());
                    }
                    ui.label(RichText::new(self.text).color(color));
                });
            });
    }
}

/// Scrollable container with consistent styling
pub struct ScrollableContainer {
    max_height: Option<f32>,
    show_scrollbar: bool,
}

#[allow(dead_code)]
impl Default for ScrollableContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollableContainer {
    pub fn new() -> Self {
        Self {
            max_height: None,
            show_scrollbar: true,
        }
    }

    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    pub fn show_scrollbar(mut self, show: bool) -> Self {
        self.show_scrollbar = show;
        self
    }

    pub fn show<R>(self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        let mut scroll = egui::ScrollArea::vertical();

        if let Some(height) = self.max_height {
            scroll = scroll.max_height(height);
        }

        if !self.show_scrollbar {
            scroll =
                scroll.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        }

        scroll.show(ui, content).inner
    }
}

/// Styled checkbox with Dash theme
pub struct StyledCheckbox<'a> {
    checked: &'a mut bool,
    text: String,
}

#[allow(dead_code)]
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
pub struct GradientButton {
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

/// Glass-morphism styled card
pub struct GlassCard {
    title: Option<String>,
    padding: f32,
}

#[allow(dead_code)]
impl Default for GlassCard {
    fn default() -> Self {
        Self::new()
    }
}

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
        egui::Frame::new()
            .fill(DashColors::glass_white())
            .stroke(Stroke::new(1.0, DashColors::glass_border()))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_XL))
            .inner_margin(egui::Margin::same(self.padding as i8))
            .shadow(Shadow::medium())
            .show(ui, |ui| {
                if let Some(title) = self.title {
                    ui.label(
                        RichText::new(title)
                            .font(Typography::heading_medium())
                            .color(DashColors::TEXT_PRIMARY),
                    );
                    ui.add_space(Spacing::MD);
                }
                content(ui)
            })
            .inner
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
                    ui.label(
                        RichText::new(self.title)
                            .font(Typography::heading_large())
                            .color(DashColors::TEXT_PRIMARY),
                    );

                    if let Some(subtitle) = self.subtitle {
                        ui.add_space(Spacing::SM);
                        ui.label(
                            RichText::new(subtitle)
                                .font(Typography::body_large())
                                .color(DashColors::TEXT_SECONDARY),
                        );
                    }
                });
            });

        // Request repaint for animation
        repaint_animation(ui.ctx(), result.response.id);
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
            repaint_animation(ui.ctx(), response.id);
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
impl Default for AnimatedGradientCard {
    fn default() -> Self {
        Self::new()
    }
}

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
                repaint_animation(ui.ctx(), ui.id());

                content(ui)
            })
            .inner
    }
}

/// Helper function to style a TextEdit with consistent theme
pub fn styled_text_edit_singleline(text: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(text).background_color(DashColors::INPUT_BACKGROUND)
}

/// Helper function to style a multiline TextEdit with consistent theme
#[allow(dead_code)]
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
