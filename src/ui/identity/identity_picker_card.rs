//! Identity picker card — one card per identity in the picker grid.
//!
//! Design reference: `docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md`
//! §B.14 (Identity picker, Frame 2). The card displays:
//!
//! * A 72×72 circular avatar or monogram glyph (rendered via a colored circle
//!   with an initial letter — avatar assets land in a follow-up task).
//! * An identity-type badge pill anchored near the top.
//! * A heading line using the priority:
//!   `display_name → DPNS handle → shortened Identity ID`.
//! * A sub-line:
//!   * `@{dpns_handle}` when a DPNS handle exists AND the heading is NOT the
//!     DPNS handle (i.e. display_name is set and distinct).
//!   * The identity-type label (e.g. `User identity`) otherwise.
//! * A balance line rendered with tabular numerals, with an optional fiat line
//!   below (the fiat conversion is supplied by the caller — this component
//!   only renders what it is given).
//! * An "Opens Identity Home →" wireframe chip at the bottom, rendered as a
//!   small info-colored hint.
//!
//! The whole card is a single click target.
//!
//! Follows the project Component Design Pattern
//! (`docs/COMPONENT_DESIGN_PATTERN.md`): private fields, builder methods,
//! `show()` returns a typed response implementing [`ComponentResponse`].

use super::identity_pill::shorten_id;
use crate::model::qualified_identity::IdentityType;
use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::theme::DashColors;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Stroke, Ui,
    Vec2, WidgetInfo, WidgetType,
};

/// Fixed card width used by the grid layout. Matches the design-spec
/// `minmax(260px, 1fr)` rule for the picker grid.
pub const CARD_MIN_WIDTH: f32 = 260.0;

/// Approximate card height — used by the grid when it allocates rows. Actual
/// content may expand slightly; the frame fill absorbs any leftover space.
pub const CARD_HEIGHT: f32 = 220.0;

/// Avatar / monogram diameter (design-spec: 72×72).
const AVATAR_SIZE: f32 = 72.0;

/// Resolve the heading shown on the identity picker card.
///
/// Priority:
/// 1. `display_name` (social-profile display name)
/// 2. `dpns_handle` (primary DPNS username)
/// 3. shortened Identity ID (`Fx1Kj…9Tt` style, see [`shorten_id`]).
///
/// When every source is missing or empty the heading falls back to the string
/// `"Unknown identity"` — a safe, i18n-ready sentence fragment so the card is
/// never blank.
pub fn card_heading(
    display_name: Option<&str>,
    dpns_handle: Option<&str>,
    identity_id_base58: &str,
) -> String {
    if let Some(name) = display_name.map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    if let Some(handle) = dpns_handle.map(str::trim).filter(|s| !s.is_empty()) {
        return handle.to_string();
    }
    let trimmed = identity_id_base58.trim();
    if trimmed.is_empty() {
        return "Unknown identity".to_string();
    }
    shorten_id(trimmed)
}

/// Resolve the sub-line shown beneath the heading.
///
/// * Returns `@{dpns_handle}` when a DPNS handle exists AND the heading is
///   NOT the DPNS handle (i.e. the display name provides the heading).
/// * Otherwise returns the identity-type label
///   (`User identity` / `Masternode identity` / `Evonode identity`).
///
/// This mirrors the test matrix in UT-PICKER-01 / UT-PICKER-02.
pub fn card_sub_line(
    display_name: Option<&str>,
    dpns_handle: Option<&str>,
    identity_type: IdentityType,
) -> String {
    let has_display_name = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    let handle = dpns_handle.map(str::trim).filter(|s| !s.is_empty());

    if has_display_name && let Some(h) = handle {
        return format!("@{h}");
    }

    identity_type_label(identity_type).to_string()
}

/// Identity-type label string used when no DPNS handle / display-name sub-line
/// is available. Complete i18n-ready sentence fragment per the project style
/// guide — never concatenate.
pub fn identity_type_label(identity_type: IdentityType) -> &'static str {
    match identity_type {
        IdentityType::User => "User identity",
        IdentityType::Masternode => "Masternode identity",
        IdentityType::Evonode => "Evonode identity",
    }
}

/// Response returned by [`IdentityPickerCard::show`].
///
/// Reports whether the card was activated. When activated, `changed_value`
/// carries the identity id string supplied to the card so a grid composer can
/// route selection without tracking indices.
#[derive(Clone, Debug)]
pub struct IdentityPickerCardResponse {
    /// Whether the user clicked the card.
    pub clicked: bool,
    /// The identity id associated with the card (echoed from construction).
    pub identity_id: String,
    changed_value: Option<String>,
}

impl IdentityPickerCardResponse {
    pub(crate) fn new(identity_id: String, clicked: bool) -> Self {
        let changed_value = if clicked {
            Some(identity_id.clone())
        } else {
            None
        };
        Self {
            clicked,
            identity_id,
            changed_value,
        }
    }
}

impl ComponentResponse for IdentityPickerCardResponse {
    type DomainType = String;

    fn has_changed(&self) -> bool {
        self.clicked
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

/// Identity picker card widget. See module docs for the visual design.
#[derive(Clone, Debug)]
pub struct IdentityPickerCard {
    identity_id_base58: String,
    display_name: Option<String>,
    dpns_handle: Option<String>,
    identity_type: IdentityType,
    /// Formatted balance string (e.g. `0.75 DASH`). The component does not
    /// format credits itself — callers decide the unit + precision.
    balance_label: String,
    /// Optional fiat conversion line (e.g. `≈ 45.25 USD`). Empty = not shown.
    fiat_label: String,
    tooltip: String,
}

impl IdentityPickerCard {
    /// Build a picker card.
    ///
    /// `identity_id_base58` is echoed back in the response on click so callers
    /// can route the selection without tracking indices.
    pub fn new(
        identity_id_base58: impl Into<String>,
        identity_type: IdentityType,
        balance_label: impl Into<String>,
    ) -> Self {
        Self {
            identity_id_base58: identity_id_base58.into(),
            display_name: None,
            dpns_handle: None,
            identity_type,
            balance_label: balance_label.into(),
            fiat_label: String::new(),
            tooltip: String::new(),
        }
    }

    /// Attach a display name (social-profile display name). Takes priority
    /// over the DPNS handle for the heading line.
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Attach a DPNS handle (without the leading `@`).
    pub fn with_dpns_handle(mut self, handle: impl Into<String>) -> Self {
        self.dpns_handle = Some(handle.into());
        self
    }

    /// Attach an optional fiat conversion line.
    pub fn with_fiat_label(mut self, fiat: impl Into<String>) -> Self {
        self.fiat_label = fiat.into();
        self
    }

    /// Attach a tooltip (shown on hover).
    pub fn with_tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = text.into();
        self
    }

    /// Compute the heading string — exposed for tests (UT-PICKER-01 / 02).
    pub fn heading(&self) -> String {
        card_heading(
            self.display_name.as_deref(),
            self.dpns_handle.as_deref(),
            &self.identity_id_base58,
        )
    }

    /// Compute the sub-line string — exposed for tests.
    pub fn sub_line(&self) -> String {
        card_sub_line(
            self.display_name.as_deref(),
            self.dpns_handle.as_deref(),
            self.identity_type,
        )
    }

    /// The identity-type badge pill text.
    pub fn badge_label(&self) -> &'static str {
        match self.identity_type {
            IdentityType::User => "User",
            IdentityType::Masternode => "Masternode",
            IdentityType::Evonode => "Evonode",
        }
    }

    /// Render and return the response.
    pub fn show(&self, ui: &mut Ui) -> IdentityPickerCardResponse {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        let border = Stroke::new(1.0, DashColors::border(dark_mode));
        let fill = DashColors::surface(dark_mode);
        let heading = self.heading();
        let sub_line = self.sub_line();
        let badge_label = self.badge_label();

        let heading_for_a11y = heading.clone();

        let frame = Frame::new()
            .fill(fill)
            .stroke(border)
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(16, 16));

        // Pre-allocate a fixed-size region so every card in the grid has a
        // predictable footprint. The frame stretches to fill.
        let desired_size = Vec2::new(CARD_MIN_WIDTH, CARD_HEIGHT);

        // `Frame::show` senses only hover on the outer allocation; to pick up
        // clicks we read the response from an interact() call on the frame's
        // rect after the content renders.
        let inner = frame.show(ui, |ui| {
            ui.set_min_size(desired_size);
            ui.set_max_width(desired_size.x);
            ui.vertical(|ui| {
                // Top row: avatar (left) + badge (right).
                ui.horizontal(|ui| {
                    draw_monogram(ui, &heading, self.display_name.is_some(), dark_mode);
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        draw_type_badge(ui, badge_label, dark_mode);
                    });
                });
                ui.add_space(12.0);

                // Heading.
                ui.label(
                    RichText::new(&heading)
                        .color(DashColors::text_primary(dark_mode))
                        .strong()
                        .size(16.0),
                );
                // Sub-line.
                ui.label(
                    RichText::new(&sub_line)
                        .color(DashColors::text_secondary(dark_mode))
                        .size(13.0),
                );
                ui.add_space(8.0);

                // Balance (tabular numerals).
                ui.label(
                    RichText::new(&self.balance_label)
                        .color(DashColors::text_primary(dark_mode))
                        .strong()
                        .font(FontId::monospace(13.0)),
                );
                if !self.fiat_label.is_empty() {
                    ui.label(
                        RichText::new(&self.fiat_label)
                            .color(DashColors::text_secondary(dark_mode))
                            .font(FontId::monospace(11.0)),
                    );
                }

                ui.add_space(8.0);

                // Bottom wireframe hint chip. Using a small muted label rather
                // than a dedicated chip component — matches the design-spec
                // note that this is a wireframe affordance, not a full chip.
                ui.label(
                    RichText::new("Opens Identity Home →")
                        .color(DashColors::info_color(dark_mode))
                        .size(11.0),
                );
            });
        });

        // Make the entire card interactive — one click target for the whole
        // surface, per §B.14.
        let rect = inner.response.rect;
        let id = ui
            .id()
            .with(("identity-picker-card", &self.identity_id_base58));
        let response: Response = ui.interact(rect, id, Sense::click());
        let response = if !self.tooltip.is_empty() {
            response.on_hover_text(&self.tooltip)
        } else {
            response
        };
        // Hover elevation: repaint a subtle secondary border when hovered.
        if response.hovered() {
            let painter = ui.painter();
            painter.rect_stroke(
                rect,
                CornerRadius::same(16),
                Stroke::new(1.5, DashColors::border_light(dark_mode)),
                egui::StrokeKind::Inside,
            );
        }
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, format!("Open {heading_for_a11y}"))
        });

        IdentityPickerCardResponse::new(self.identity_id_base58.clone(), response.clicked())
    }
}

/// Paint a simple circular monogram as a lightweight avatar stand-in. Real
/// avatar assets land in a follow-up task (see design-spec §B.14).
fn draw_monogram(ui: &mut Ui, heading: &str, has_social_profile: bool, dark_mode: bool) {
    let (rect, _response) =
        ui.allocate_exact_size(Vec2::new(AVATAR_SIZE, AVATAR_SIZE), Sense::hover());
    let painter = ui.painter();

    let fill_color = if has_social_profile {
        DashColors::DASH_BLUE
    } else {
        DashColors::surface_elevated(dark_mode)
    };
    let text_color = if has_social_profile {
        Color32::WHITE
    } else {
        DashColors::text_secondary(dark_mode)
    };

    painter.circle_filled(rect.center(), AVATAR_SIZE / 2.0, fill_color);
    // Thin ring for the non-social variant so it reads as an empty slot, not a
    // missing fill.
    if !has_social_profile {
        painter.circle_stroke(
            rect.center(),
            AVATAR_SIZE / 2.0,
            Stroke::new(1.0, DashColors::border(dark_mode)),
        );
    }

    let initial = heading
        .chars()
        .next()
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .unwrap_or('?');
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        FontId::proportional(28.0),
        text_color,
    );
}

/// Paint an identity-type badge pill. Color follows the identity-type.
fn draw_type_badge(ui: &mut Ui, label: &str, dark_mode: bool) {
    let (fill, stroke_color) = match label {
        "Masternode" => (DashColors::PLATFORM_PURPLE, DashColors::PLATFORM_PURPLE),
        "Evonode" => (DashColors::DASH_BLUE, DashColors::DASH_BLUE),
        _ => (
            DashColors::surface_elevated(dark_mode),
            DashColors::border(dark_mode),
        ),
    };
    let text_color = if matches!(label, "Masternode" | "Evonode") {
        Color32::WHITE
    } else {
        DashColors::text_primary(dark_mode)
    };

    let frame = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke_color))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(8, 2));
    frame.show(ui, |ui| {
        ui.label(RichText::new(label).color(text_color).size(11.0).strong());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_picker_01_heading_and_sub_line_with_display_name_and_dpns() {
        // UT-PICKER-01: display_name=Some("Alex"), dpns=Some("alex.dash").
        // Heading must be "Alex"; sub-line must be "@alex.dash".
        let heading = card_heading(Some("Alex"), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(heading, "Alex");
        let sub = card_sub_line(Some("Alex"), Some("alex.dash"), IdentityType::User);
        assert_eq!(sub, "@alex.dash");
    }

    #[test]
    fn ut_picker_02_heading_and_sub_line_without_display_name() {
        // UT-PICKER-02: display_name=None, dpns=Some("mn-east-01.dash").
        // Heading must be the handle; sub-line must be the identity-type label.
        let heading = card_heading(None, Some("mn-east-01.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(heading, "mn-east-01.dash");
        let sub = card_sub_line(None, Some("mn-east-01.dash"), IdentityType::Masternode);
        assert_eq!(sub, "Masternode identity");
    }

    #[test]
    fn heading_falls_back_to_shortened_id() {
        let heading = card_heading(None, None, "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(heading, "Fx1Kj…9Tt");
    }

    #[test]
    fn sub_line_uses_identity_type_when_no_dpns() {
        let sub = card_sub_line(None, None, IdentityType::User);
        assert_eq!(sub, "User identity");
        let sub = card_sub_line(None, None, IdentityType::Evonode);
        assert_eq!(sub, "Evonode identity");
    }

    #[test]
    fn sub_line_uses_identity_type_when_heading_is_dpns() {
        // Sub-line rule: @handle only when heading comes from display_name.
        // When the DPNS handle is the heading, sub-line must NOT repeat it.
        let sub = card_sub_line(None, Some("mn-east-01.dash"), IdentityType::Masternode);
        assert_eq!(sub, "Masternode identity");
    }

    #[test]
    fn empty_heading_fallback_is_stable_sentence_fragment() {
        let heading = card_heading(None, None, "");
        assert_eq!(heading, "Unknown identity");
    }

    #[test]
    fn whitespace_display_name_falls_through_to_dpns() {
        let heading = card_heading(Some("   "), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(heading, "alex.dash");
    }

    #[test]
    fn builder_chain_stores_all_fields() {
        let card = IdentityPickerCard::new("Fx1Kj9TtFx1Kj9Tt", IdentityType::User, "0.75 DASH")
            .with_display_name("Alex")
            .with_dpns_handle("alex.dash")
            .with_fiat_label("≈ 45.25 USD")
            .with_tooltip("Open Alex");
        assert_eq!(card.heading(), "Alex");
        assert_eq!(card.sub_line(), "@alex.dash");
        assert_eq!(card.badge_label(), "User");
        assert_eq!(card.balance_label, "0.75 DASH");
        assert_eq!(card.fiat_label, "≈ 45.25 USD");
        assert_eq!(card.tooltip, "Open Alex");
    }

    #[test]
    fn response_clicked_populates_changed_value() {
        let resp = IdentityPickerCardResponse::new("abc".to_string(), true);
        assert!(resp.has_changed());
        assert!(resp.is_valid());
        assert_eq!(resp.changed_value().as_deref(), Some("abc"));
        assert_eq!(resp.identity_id, "abc");
    }

    #[test]
    fn response_not_clicked_has_no_change() {
        let resp = IdentityPickerCardResponse::new("abc".to_string(), false);
        assert!(!resp.has_changed());
        assert!(resp.changed_value().is_none());
    }

    #[test]
    fn response_update_writes_to_option() {
        let resp = IdentityPickerCardResponse::new("abc".to_string(), true);
        let mut selected: Option<String> = None;
        assert!(resp.update(&mut selected));
        assert_eq!(selected.as_deref(), Some("abc"));
    }
}
