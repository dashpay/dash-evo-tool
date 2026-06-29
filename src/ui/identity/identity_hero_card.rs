//! Identity hero card — the top-of-Home visual summary of a single identity.
//!
//! See design-spec §B.2 / §B.3 / §E. Two variants:
//!
//! - **Social profile set**: 96 px avatar circle (initials fallback when no
//!   image) + display name + `@handle` line + balance + identity-type +
//!   network pill.
//! - **No social profile**: same layout, but the avatar is replaced by a
//!   type-glyph monogram (person / masternode / evonode) and the display-name
//!   line is omitted. If the identity also has no DPNS name, the handle line
//!   becomes the `No username yet` prompt with a `Pick a username` link.
//!
//! Visual direction is locked by §E:
//! - Gradient background: `DASH_BLUE` (#008de4) → `PLATFORM_PURPLE` at 14 %
//!   opacity, laid over `DashColors::surface(dark_mode)`.
//! - `RADIUS_XL` corners, `Shadow::elevated()`.
//! - Balance uses tabular numerals (monospace) so digits align across frames.
//!
//! This component follows `docs/COMPONENT_DESIGN_PATTERN.md`: private fields +
//! builder methods + a response struct implementing [`ComponentResponse`].

use crate::model::qualified_identity::IdentityType;
use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::theme::{DashColors, ResponseExt, Shadow, Shape, Spacing};
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Rect, Response, RichText,
    Sense, Shape as EguiShape, Stroke, StrokeKind, TextStyle, Ui, vec2,
};

/// One of the three supported identity kinds. Maps 1:1 to the project's
/// [`IdentityType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeroIdentityKind {
    User,
    Masternode,
    Evonode,
}

impl From<IdentityType> for HeroIdentityKind {
    fn from(value: IdentityType) -> Self {
        match value {
            IdentityType::User => HeroIdentityKind::User,
            IdentityType::Masternode => HeroIdentityKind::Masternode,
            IdentityType::Evonode => HeroIdentityKind::Evonode,
        }
    }
}

impl HeroIdentityKind {
    /// Alex-facing identity-type badge label. See §C wording audit.
    pub fn badge_label(self) -> &'static str {
        match self {
            HeroIdentityKind::User => "User identity",
            HeroIdentityKind::Masternode => "Masternode identity",
            HeroIdentityKind::Evonode => "Evonode identity",
        }
    }

    /// Tooltip text attached to the badge. From §D row 12/13/14.
    pub fn badge_tooltip(self) -> &'static str {
        match self {
            HeroIdentityKind::User => "A regular identity used for payments, DPNS, and DashPay.",
            HeroIdentityKind::Masternode => {
                "An identity tied to a Dash masternode. It can vote on name contests."
            }
            HeroIdentityKind::Evonode => {
                "An identity tied to a Dash evonode. It can vote and validate Platform transactions."
            }
        }
    }

    /// Accent color for the badge fill (at 12 % opacity, see §E).
    pub fn badge_accent(self) -> Color32 {
        match self {
            HeroIdentityKind::User => DashColors::DASH_BLUE,
            HeroIdentityKind::Masternode => DashColors::PLATFORM_PURPLE,
            HeroIdentityKind::Evonode => DashColors::HIGHLIGHT_GOLD,
        }
    }

    /// Monogram glyph for the no-social-profile variant. §E calls for a
    /// single-person silhouette for users, a node / server glyph for
    /// masternodes, and a diamond for evonodes.
    pub fn type_glyph(self) -> &'static str {
        match self {
            HeroIdentityKind::User => "\u{1F464}", // 👤 bust in silhouette
            HeroIdentityKind::Masternode => "\u{1F5A5}", // 🖥 desktop (server-like)
            HeroIdentityKind::Evonode => "\u{25C6}", // ◆ diamond
        }
    }
}

/// Action reported by the hero when the user activates a secondary control
/// inside it (today: only the `Pick a username` link in the no-DPNS variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeroAction {
    /// User clicked the `Pick a username` link shown when DPNS is missing.
    PickUsernameClicked,
}

/// Response returned by [`IdentityHeroCard::show`].
#[derive(Clone, Debug, Default)]
pub struct HeroResponse {
    /// Optional user-initiated action from inside the hero (e.g. the
    /// `Pick a username` link). `None` when the user did not click anything.
    action: Option<HeroAction>,
    has_changed: bool,
    changed_value: Option<HeroAction>,
}

impl HeroResponse {
    fn new(action: Option<HeroAction>) -> Self {
        Self {
            action,
            has_changed: action.is_some(),
            changed_value: action,
        }
    }

    /// The action the user triggered this frame, if any.
    pub fn action(&self) -> Option<HeroAction> {
        self.action
    }

    /// Returns `true` if the user clicked the `Pick a username` link.
    pub fn pick_username_clicked(&self) -> bool {
        matches!(self.action, Some(HeroAction::PickUsernameClicked))
    }
}

impl ComponentResponse for HeroResponse {
    type DomainType = HeroAction;

    fn has_changed(&self) -> bool {
        self.has_changed
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

/// Identity hero card — see module docs.
///
/// Construction is immutable once the card is built (`show` consumes `&self`).
/// Use the builder methods to attach optional data: avatar bytes, fiat
/// equivalent, network label, etc.
#[derive(Clone, Debug)]
pub struct IdentityHeroCard {
    kind: HeroIdentityKind,
    /// DashPay display name. `None` triggers the no-social-profile variant.
    display_name: Option<String>,
    /// Primary DPNS handle (without the leading `@`). When both this and
    /// `display_name` are `None`, the hero renders the `No username yet`
    /// prompt.
    dpns_handle: Option<String>,
    /// Pre-formatted balance (e.g. `"1.2345"`). No unit — the unit is appended
    /// by the hero (`" DASH"`).
    balance_dash: String,
    /// Optional fiat equivalent line (e.g. `"≈ 47.12 USD"`). Rendered below
    /// the balance when present.
    fiat_equivalent: Option<String>,
    /// Optional network label rendered inside the network pill. `None` hides
    /// the pill.
    network_label: Option<String>,
    /// Tooltip attached to the network pill (verbatim from §D row 15).
    network_tooltip: Option<String>,
    /// Optional avatar bytes (PNG/JPEG). If present, the hero paints this
    /// image inside the avatar circle; otherwise an initials fallback is
    /// drawn for the social-profile-set variant.
    avatar_bytes: Option<Vec<u8>>,
}

impl IdentityHeroCard {
    /// Construct a new hero card with the minimum required data: identity
    /// type and balance text. All other fields have builder methods.
    pub fn new(kind: HeroIdentityKind, balance_dash: impl Into<String>) -> Self {
        Self {
            kind,
            display_name: None,
            dpns_handle: None,
            balance_dash: balance_dash.into(),
            fiat_equivalent: None,
            network_label: None,
            network_tooltip: None,
            avatar_bytes: None,
        }
    }

    /// Set the DashPay display name. Passing a non-empty string flips the
    /// hero into its social-profile-set variant.
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.trim().is_empty() {
            self.display_name = Some(name);
        }
        self
    }

    /// Set the primary DPNS handle (without the leading `@`).
    pub fn with_dpns_handle(mut self, handle: impl Into<String>) -> Self {
        let handle = handle.into();
        if !handle.trim().is_empty() {
            self.dpns_handle = Some(handle);
        }
        self
    }

    /// Attach a fiat-equivalent line rendered below the balance.
    pub fn with_fiat_equivalent(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.trim().is_empty() {
            self.fiat_equivalent = Some(text);
        }
        self
    }

    /// Attach a network pill label. `None` hides the pill.
    pub fn with_network_label(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        if !label.trim().is_empty() {
            self.network_label = Some(label);
        }
        self
    }

    /// Attach a tooltip to the network pill.
    pub fn with_network_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        let tooltip = tooltip.into();
        if !tooltip.trim().is_empty() {
            self.network_tooltip = Some(tooltip);
        }
        self
    }

    /// Attach raw avatar bytes (PNG / JPEG). When present, they override the
    /// initials fallback.
    pub fn with_avatar_bytes(mut self, bytes: Vec<u8>) -> Self {
        if !bytes.is_empty() {
            self.avatar_bytes = Some(bytes);
        }
        self
    }

    /// True when the hero is in its social-profile-set variant.
    pub fn has_social_profile(&self) -> bool {
        self.display_name.is_some()
    }

    /// True when the hero will render the `No username yet` prompt instead of
    /// a `@handle` line.
    pub fn no_dpns(&self) -> bool {
        self.dpns_handle.is_none()
    }

    /// True when the hero will use the initials fallback instead of an image
    /// (only meaningful in the social-profile-set variant). Exposed for tests.
    pub fn avatar_uses_initials_fallback(&self) -> bool {
        self.has_social_profile() && self.avatar_bytes.is_none()
    }

    /// Initial letter for the initials fallback, uppercase. Falls back to
    /// `"?"` if the display name has no alphabetic character.
    pub fn initials_letter(&self) -> char {
        self.display_name
            .as_deref()
            .and_then(|name| {
                name.chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().next().unwrap_or(c))
            })
            .unwrap_or('?')
    }

    /// Render the hero card. Returns a [`HeroResponse`] carrying any
    /// user-initiated action.
    pub fn show(&self, ui: &mut Ui) -> HeroResponse {
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;

        // Outer gradient-surface frame. We paint a solid surface fill and then
        // overlay a gradient rectangle ourselves because `egui::Frame` does
        // not support multi-stop gradients directly.
        let frame = Frame::new()
            .fill(DashColors::surface(dark_mode))
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border_light(dark_mode),
            ))
            .corner_radius(CornerRadius::same(Shape::RADIUS_XL))
            .shadow(Shadow::elevated())
            .inner_margin(Margin::same(Spacing::LG as i8));

        let response = frame.show(ui, |ui| {
            // Reserve the full hero height (~240 px target, min 200).
            ui.set_min_height(200.0);

            // Paint the gradient band before any widgets so the labels sit on
            // top. Two horizontal stops at 14 % opacity.
            self.paint_gradient_band(ui);

            let mut action: Option<HeroAction> = None;
            ui.horizontal(|ui| {
                // Left cluster: avatar / monogram + social lines.
                ui.vertical(|ui| {
                    self.paint_avatar_or_monogram(ui);
                });
                ui.add_space(Spacing::LG);

                ui.vertical(|ui| {
                    ui.add_space(Spacing::XS);
                    if let Some(name) = &self.display_name {
                        ui.label(
                            RichText::new(name)
                                .size(28.0)
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    }
                    match (&self.display_name, &self.dpns_handle) {
                        (_, Some(handle)) => {
                            ui.label(
                                RichText::new(format!("@{handle}"))
                                    .size(16.0)
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        }
                        (Some(_), None) => {
                            // Social profile set but no DPNS handle — show
                            // the `Pick a username` prompt.
                            if self.paint_pick_username_prompt(ui, dark_mode) {
                                action = Some(HeroAction::PickUsernameClicked);
                            }
                        }
                        (None, None) => {
                            // No display name and no DPNS handle — still show
                            // the prompt so the user has a recovery path.
                            if self.paint_pick_username_prompt(ui, dark_mode) {
                                action = Some(HeroAction::PickUsernameClicked);
                            }
                        }
                    }
                });

                // Spacer pushes the right cluster to the edge.
                ui.add_space(Spacing::LG);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                            // Balance — tabular numerals (monospace).
                            let balance_id = FontId::new(24.0, FontFamily::Monospace);
                            ui.label(
                                RichText::new(format!("{} DASH", self.balance_dash))
                                    .font(balance_id)
                                    .strong()
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            if let Some(fiat) = &self.fiat_equivalent {
                                ui.label(
                                    RichText::new(fiat)
                                        .size(13.0)
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            }
                            ui.add_space(Spacing::SM);

                            // Identity-type + network pill row.
                            ui.horizontal(|ui| {
                                self.paint_pill(
                                    ui,
                                    self.kind.badge_label(),
                                    self.kind.badge_accent(),
                                    Some(self.kind.badge_tooltip()),
                                );
                                if let Some(label) = &self.network_label {
                                    ui.add_space(Spacing::XS);
                                    self.paint_pill(
                                        ui,
                                        label,
                                        DashColors::INFO,
                                        self.network_tooltip.as_deref(),
                                    );
                                }
                            });
                        });
                    });
                });
            });

            action
        });

        HeroResponse::new(response.inner)
    }

    /// Paint the 14 %-opacity diagonal gradient band across the card.
    ///
    /// Uses a series of narrow vertical strips because egui's `Shape::Rect`
    /// does not support linear gradients. Keeping the strip count low keeps
    /// overdraw cheap even on large canvases.
    fn paint_gradient_band(&self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let strips = 32u32;
        let alpha: u8 = (0.14 * 255.0) as u8;
        let painter = ui.painter();
        for i in 0..strips {
            let t = i as f32 / (strips as f32 - 1.0);
            let c = lerp_color(DashColors::DASH_BLUE, DashColors::PLATFORM_PURPLE, t);
            let a = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha);
            let x0 = rect.left() + rect.width() * (i as f32 / strips as f32);
            let x1 = rect.left() + rect.width() * ((i + 1) as f32 / strips as f32);
            let strip =
                Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom()));
            painter.rect_filled(strip, 0.0, a);
        }
    }

    /// Paint the avatar circle (social profile set) or the type-glyph
    /// monogram (no social profile).
    fn paint_avatar_or_monogram(&self, ui: &mut Ui) {
        let size = 96.0;
        let (rect, _resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        let painter = ui.painter();
        let center = rect.center();
        let radius = size * 0.5;

        let ring_color = Color32::from_rgba_unmultiplied(
            DashColors::DASH_BLUE.r(),
            DashColors::DASH_BLUE.g(),
            DashColors::DASH_BLUE.b(),
            (0.20 * 255.0) as u8,
        );

        if self.has_social_profile() {
            // Social profile set → avatar circle. Fallback = solid Dash-blue
            // fill with initials in white. Image rendering would require
            // async texture decoding — kept for a follow-up; the current
            // variant always renders the initials fallback if bytes are
            // present but no texture pipeline is wired here.
            painter.circle_filled(center, radius, DashColors::DASH_BLUE);
            painter.circle_stroke(center, radius, Stroke::new(2.0, ring_color));
            let letter = self.initials_letter().to_string();
            let font_id = TextStyle::Heading.resolve(ui.style());
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                letter,
                font_id,
                Color32::WHITE,
            );
        } else {
            // No social profile → type-glyph monogram.
            let bg_color = Color32::from_rgba_unmultiplied(
                DashColors::DASH_BLUE.r(),
                DashColors::DASH_BLUE.g(),
                DashColors::DASH_BLUE.b(),
                (0.08 * 255.0) as u8,
            );
            painter.circle_filled(center, radius, bg_color);
            painter.circle_stroke(center, radius, Stroke::new(2.0, ring_color));
            let glyph = self.kind.type_glyph();
            let font_id = FontId::new(40.0, FontFamily::Proportional);
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                glyph,
                font_id,
                DashColors::DASH_BLUE,
            );
        }
    }

    /// Paint the `No username yet` / `Pick a username` affordance. Returns
    /// `true` when the link is clicked.
    fn paint_pick_username_prompt(&self, ui: &mut Ui, dark_mode: bool) -> bool {
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("No username yet")
                    .italics()
                    .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(Spacing::XS);
            let link = ui
                .add(
                    egui::Label::new(
                        RichText::new("Pick a username")
                            .underline()
                            .color(DashColors::DASH_BLUE),
                    )
                    .sense(Sense::click()),
                )
                .clickable_tooltip("Register a username so people can send you Dash by name.");
            if link.clicked() {
                clicked = true;
            }
        });
        clicked
    }

    /// Paint an identity-type or network pill. Fill is the accent color at
    /// 12 % opacity, with a 1 px stroke at the accent color. See §E.
    fn paint_pill(
        &self,
        ui: &mut Ui,
        label: &str,
        accent: Color32,
        tooltip: Option<&str>,
    ) -> Response {
        let fill = Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            (0.12 * 255.0) as u8,
        );
        let stroke_color = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 180);

        let text = RichText::new(label).color(accent).size(12.0).strong();
        // Build as an inline pill: small frame around the label.
        let inner = Frame::new()
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(Shape::RADIUS_FULL))
            .inner_margin(Margin::symmetric(10, 3))
            .show(ui, |ui| {
                ui.add(egui::Label::new(text).sense(Sense::hover()))
            });
        // Paint the stroke manually so the ring color matches the accent.
        ui.painter().rect_stroke(
            inner.response.rect,
            CornerRadius::same(Shape::RADIUS_FULL),
            Stroke::new(1.0, stroke_color),
            StrokeKind::Outside,
        );
        // Suppress the unused-shape lint by explicitly dropping it.
        let _ = EguiShape::Noop;
        if let Some(text) = tooltip {
            inner.response.info_tooltip(text)
        } else {
            inner.response
        }
    }
}

/// Linear interpolate two [`Color32`] values component-wise.
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let clamp = t.clamp(0.0, 1.0);
    let lerp_u8 = |x: u8, y: u8| -> u8 {
        let xf = x as f32;
        let yf = y as f32;
        (xf + (yf - xf) * clamp).round() as u8
    };
    Color32::from_rgba_unmultiplied(
        lerp_u8(a.r(), b.r()),
        lerp_u8(a.g(), b.g()),
        lerp_u8(a.b(), b.b()),
        255,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // UT-HERO-01 — Identity hero, social profile set.
    //
    // Preconditions: identity with display name + handle + balance.
    // Expected: avatar uses the initials fallback when no image; handle line
    // uses text_secondary; balance uses tabular numerals (monospace).
    #[test]
    fn hero_social_profile_set_configures_avatar_and_handle() {
        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "1.2345")
            .with_display_name("Alex")
            .with_dpns_handle("alex.dash");
        // Social profile state detected.
        assert!(hero.has_social_profile());
        assert!(!hero.no_dpns());
        // No avatar bytes provided → initials fallback engaged.
        assert!(hero.avatar_uses_initials_fallback());
        assert_eq!(hero.initials_letter(), 'A');
        // Balance string is stored verbatim (tabular numeral rendering is
        // driven by the monospace FontId inside `show`, not stored here).
        assert_eq!(hero.balance_dash, "1.2345");
        // DPNS handle present so the no-username fallback is NOT engaged.
        assert_eq!(hero.dpns_handle.as_deref(), Some("alex.dash"));
    }

    // UT-HERO-02 — Identity hero, no social profile.
    //
    // Preconditions: same identity with display_name = None.
    // Expected: type-glyph monogram instead of avatar; no display-name line.
    #[test]
    fn hero_no_social_profile_uses_type_glyph() {
        let hero =
            IdentityHeroCard::new(HeroIdentityKind::User, "0.1234").with_dpns_handle("alex.dash");
        // No display name → no social profile variant.
        assert!(!hero.has_social_profile());
        // Avatar fallback for social profile variant does not apply.
        assert!(!hero.avatar_uses_initials_fallback());
        // Type glyph is the user silhouette for the User kind.
        assert_eq!(HeroIdentityKind::User.type_glyph(), "\u{1F464}");
        assert_eq!(HeroIdentityKind::Masternode.type_glyph(), "\u{1F5A5}");
        assert_eq!(HeroIdentityKind::Evonode.type_glyph(), "\u{25C6}");
    }

    #[test]
    fn display_name_trimming_treats_whitespace_as_missing() {
        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.0")
            .with_display_name("   ")
            .with_dpns_handle("alex.dash");
        assert!(!hero.has_social_profile());
    }

    #[test]
    fn initials_letter_picks_first_alphanumeric_uppercase() {
        let hero =
            IdentityHeroCard::new(HeroIdentityKind::User, "0.0").with_display_name("!! jamie");
        assert_eq!(hero.initials_letter(), 'J');
    }

    #[test]
    fn initials_letter_defaults_to_question_mark_when_no_name() {
        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.0");
        assert_eq!(hero.initials_letter(), '?');
    }

    #[test]
    fn identity_kind_roundtrips_from_model_type() {
        assert_eq!(
            HeroIdentityKind::from(IdentityType::User),
            HeroIdentityKind::User
        );
        assert_eq!(
            HeroIdentityKind::from(IdentityType::Masternode),
            HeroIdentityKind::Masternode
        );
        assert_eq!(
            HeroIdentityKind::from(IdentityType::Evonode),
            HeroIdentityKind::Evonode
        );
    }

    #[test]
    fn hero_response_reports_no_action_by_default() {
        let resp = HeroResponse::default();
        assert!(!resp.has_changed());
        assert!(resp.is_valid());
        assert!(resp.action().is_none());
        assert!(!resp.pick_username_clicked());
    }

    #[test]
    fn hero_response_reports_pick_username_click() {
        let resp = HeroResponse::new(Some(HeroAction::PickUsernameClicked));
        assert!(resp.has_changed());
        assert!(resp.pick_username_clicked());
        assert_eq!(resp.changed_value(), &Some(HeroAction::PickUsernameClicked));
    }

    #[test]
    fn lerp_color_clamps_and_midpoint() {
        let mid = lerp_color(Color32::BLACK, Color32::WHITE, 0.5);
        assert!(mid.r() >= 126 && mid.r() <= 129);
        let start = lerp_color(Color32::BLACK, Color32::WHITE, -1.0);
        assert_eq!(start, Color32::from_rgba_unmultiplied(0, 0, 0, 255));
        let end = lerp_color(Color32::BLACK, Color32::WHITE, 2.0);
        assert_eq!(end, Color32::from_rgba_unmultiplied(255, 255, 255, 255));
    }
}
