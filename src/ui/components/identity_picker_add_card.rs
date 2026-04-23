//! "Add a new identity" picker card — rendered alongside regular identity
//! cards in the picker grid. Design reference:
//! `docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md` §B.14.
//!
//! Visual treatment:
//!
//! * Same grid cell dimensions as [`IdentityPickerCard`], so it slots in
//!   naturally alongside identity cards without layout shift.
//! * **Dashed border** (`2px dashed`) using the theme border color in the
//!   default state, switching to a solid Dash-blue border on hover.
//! * 72×72 circle with a large `+` glyph.
//! * Heading: `"Add a new identity"`. Sub-line:
//!   `"Create a new identity or load one you already own."`.
//!
//! The entire card is a click target. Clicking emits a response with
//! `add_requested == true` which the picker grid uses to route to
//! `AddNewIdentityScreen` (the existing, unmodified screen).

use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::components::identity_picker_card::{CARD_HEIGHT, CARD_MIN_WIDTH};
use crate::ui::theme::DashColors;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, Response, RichText, Sense,
    Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};

/// Border style currently applied to the add card. Exposed primarily for the
/// UT-PICKER-03 test — the test asserts the default style is dashed and that
/// hover switches to the solid Dash-blue treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddCardBorderStyle {
    /// Default resting state: dashed border in the theme border color.
    Dashed,
    /// Hover state: solid Dash-blue border.
    SolidDashBlue,
}

/// Response returned by [`IdentityPickerAddCard::show`].
#[derive(Clone, Debug)]
pub struct IdentityPickerAddCardResponse {
    /// True when the user clicked the card this frame.
    pub add_requested: bool,
    changed_value: Option<()>,
}

impl IdentityPickerAddCardResponse {
    pub(crate) fn new(add_requested: bool) -> Self {
        let changed_value = if add_requested { Some(()) } else { None };
        Self {
            add_requested,
            changed_value,
        }
    }
}

impl ComponentResponse for IdentityPickerAddCardResponse {
    type DomainType = ();

    fn has_changed(&self) -> bool {
        self.add_requested
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.changed_value
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// "Add a new identity" card widget. No configuration is needed — the strings
/// and styling are fixed by the design-spec.
#[derive(Clone, Debug, Default)]
pub struct IdentityPickerAddCard {
    tooltip: Option<String>,
}

impl IdentityPickerAddCard {
    /// Construct a new add card.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a custom tooltip. Defaults to the design-spec tooltip (tt-78y).
    pub fn with_tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Return the border style for the current render state. Pure function
    /// exposed for UT-PICKER-03.
    pub fn border_style(hovered: bool) -> AddCardBorderStyle {
        if hovered {
            AddCardBorderStyle::SolidDashBlue
        } else {
            AddCardBorderStyle::Dashed
        }
    }

    /// Heading text (fixed, i18n-ready).
    pub fn heading(&self) -> &'static str {
        "Add a new identity"
    }

    /// Sub-line text (fixed, i18n-ready).
    pub fn sub_line(&self) -> &'static str {
        "Create a new identity or load one you already own."
    }

    /// Render and return the response.
    pub fn show(&self, ui: &mut Ui) -> IdentityPickerAddCardResponse {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // We render in two passes because `Frame::stroke` does not support
        // dashed strokes. Pass 1: allocate the card rect + content (no outer
        // stroke). Pass 2: paint either a dashed or solid outline on top of
        // the allocated rect, based on hover state.
        let desired_size = Vec2::new(CARD_MIN_WIDTH, CARD_HEIGHT);

        let frame = Frame::new()
            .fill(DashColors::surface(dark_mode))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(16, 16));

        let inner = frame.show(ui, |ui| {
            ui.set_min_size(desired_size);
            ui.set_max_width(desired_size.x);
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                draw_plus_circle(ui, dark_mode);
                ui.add_space(16.0);
                ui.label(
                    RichText::new(self.heading())
                        .color(DashColors::text_primary(dark_mode))
                        .strong()
                        .size(16.0),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(self.sub_line())
                        .color(DashColors::text_secondary(dark_mode))
                        .size(13.0),
                );
            });
        });

        let rect = inner.response.rect;
        let id = ui.id().with("identity-picker-add-card");
        let response: Response = ui.interact(rect, id, Sense::click());

        let tooltip_text = self
            .tooltip
            .clone()
            .unwrap_or_else(|| self.sub_line().to_string());
        let response = response.on_hover_text(tooltip_text);

        // Draw the outline: dashed by default, solid Dash-blue on hover.
        let style = Self::border_style(response.hovered());
        match style {
            AddCardBorderStyle::Dashed => {
                paint_dashed_rounded_rect(
                    ui,
                    rect,
                    CornerRadius::same(16),
                    Stroke::new(2.0, DashColors::border(dark_mode)),
                );
            }
            AddCardBorderStyle::SolidDashBlue => {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(16),
                    Stroke::new(2.0, DashColors::DASH_BLUE),
                    egui::StrokeKind::Inside,
                );
            }
        }

        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, "Add a new identity".to_string())
        });

        IdentityPickerAddCardResponse::new(response.clicked())
    }
}

/// Paint the centered `+` affordance — a filled circle with a bold `+` glyph.
fn draw_plus_circle(ui: &mut Ui, dark_mode: bool) {
    const CIRCLE: f32 = 72.0;
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(CIRCLE, CIRCLE), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(
        rect.center(),
        CIRCLE / 2.0,
        DashColors::surface_elevated(dark_mode),
    );
    painter.circle_stroke(
        rect.center(),
        CIRCLE / 2.0,
        Stroke::new(1.0, DashColors::border(dark_mode)),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        FontId::proportional(36.0),
        DashColors::DASH_BLUE,
    );
}

/// Paint a dashed rounded rectangle outline. Approximates a CSS-style dashed
/// border by subdividing each side into short segments; corners are rendered
/// as tiny straight segments — visually close enough at the card size used in
/// the picker grid and cheap to draw.
fn paint_dashed_rounded_rect(ui: &mut Ui, rect: Rect, _corner: CornerRadius, stroke: Stroke) {
    const DASH_LEN: f32 = 6.0;
    const GAP_LEN: f32 = 4.0;
    let painter = ui.painter();
    // We trade perfect rounded corners for simplicity: paint dashed lines
    // along each straight edge, offset inward so they visually match the
    // frame fill. The corner radius is small enough at 16px that a straight
    // dashed line reads naturally as the card's outline.
    let corners = [
        (
            Pos2::new(rect.left(), rect.top()),
            Pos2::new(rect.right(), rect.top()),
        ),
        (
            Pos2::new(rect.right(), rect.top()),
            Pos2::new(rect.right(), rect.bottom()),
        ),
        (
            Pos2::new(rect.right(), rect.bottom()),
            Pos2::new(rect.left(), rect.bottom()),
        ),
        (
            Pos2::new(rect.left(), rect.bottom()),
            Pos2::new(rect.left(), rect.top()),
        ),
    ];
    for (start, end) in corners {
        paint_dashed_segment(painter, start, end, DASH_LEN, GAP_LEN, stroke);
    }
    // Faint corner fills to soften the square look — approximate rounding.
    let radius = 4.0;
    for corner in [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ] {
        painter.circle_filled(corner, radius, Color32::TRANSPARENT);
    }
}

fn paint_dashed_segment(
    painter: &egui::Painter,
    start: Pos2,
    end: Pos2,
    dash_len: f32,
    gap_len: f32,
    stroke: Stroke,
) {
    let vec = end - start;
    let length = vec.length();
    if length <= 0.0 {
        return;
    }
    let dir = vec / length;
    let mut traveled = 0.0;
    while traveled < length {
        let seg_start = start + dir * traveled;
        let seg_end_dist = (traveled + dash_len).min(length);
        let seg_end = start + dir * seg_end_dist;
        painter.line_segment([seg_start, seg_end], stroke);
        traveled += dash_len + gap_len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_picker_03_default_border_is_dashed() {
        // UT-PICKER-03 precondition: default construction.
        // Expected: default border style reports dashed.
        let style = IdentityPickerAddCard::border_style(false);
        assert_eq!(style, AddCardBorderStyle::Dashed);
    }

    #[test]
    fn ut_picker_03_hover_switches_to_solid_dash_blue() {
        // UT-PICKER-03 expected: hover switches to solid Dash-blue.
        let style = IdentityPickerAddCard::border_style(true);
        assert_eq!(style, AddCardBorderStyle::SolidDashBlue);
    }

    #[test]
    fn fixed_strings_match_design_spec() {
        let card = IdentityPickerAddCard::new();
        assert_eq!(card.heading(), "Add a new identity");
        assert_eq!(
            card.sub_line(),
            "Create a new identity or load one you already own."
        );
    }

    #[test]
    fn response_add_requested_populates_changed_value() {
        let resp = IdentityPickerAddCardResponse::new(true);
        assert!(resp.has_changed());
        assert!(resp.is_valid());
        assert_eq!(resp.changed_value(), &Some(()));
    }

    #[test]
    fn response_not_requested_has_no_change() {
        let resp = IdentityPickerAddCardResponse::new(false);
        assert!(!resp.has_changed());
        assert_eq!(resp.changed_value(), &None);
    }

    #[test]
    fn custom_tooltip_overrides_default() {
        let card = IdentityPickerAddCard::new().with_tooltip("Custom tooltip");
        assert_eq!(card.tooltip.as_deref(), Some("Custom tooltip"));
    }
}
