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
//! - Flat `DashColors::surface(dark_mode)` fill.
//! - `RADIUS_XL` corners, `Shadow::elevated()`.
//! - Balance uses tabular numerals (monospace) so digits align across frames.
//!
//! This component follows `docs/COMPONENT_DESIGN_PATTERN.md`: private fields +
//! builder methods + a response struct implementing [`ComponentResponse`].

use crate::model::contested_name::PendingUsername;
use crate::model::qualified_identity::IdentityType;
use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::components::pill;
use crate::ui::theme::{DashColors, ResponseExt, Shadow, Shape, Spacing};
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, Ui,
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
    /// Optional avatar bytes (PNG/JPEG). If present AND decodable, the hero
    /// paints this image inside the avatar circle; otherwise an initials
    /// fallback is used.
    avatar_bytes: Option<Vec<u8>>,
    /// `true` when `avatar_bytes` decoded successfully (checked eagerly in
    /// `with_avatar_bytes`). `false` on decode failure so that
    /// `avatar_uses_initials_fallback()` stays honest even without a render
    /// pass.
    avatar_decode_ok: bool,
    /// A DPNS username the identity has requested but not yet been awarded.
    /// When set and no owned `dpns_handle` exists, the hero shows the requested
    /// name with a "Pending" pill instead of the `No username yet` prompt.
    pending_username: Option<PendingUsername>,
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
            avatar_decode_ok: false,
            pending_username: None,
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

    /// Attach a pending DPNS username request (requested but not yet awarded).
    /// Shown only when the identity has no owned `dpns_handle` — an owned name
    /// always wins.
    pub fn with_pending_username(mut self, pending: PendingUsername) -> Self {
        if !pending.name.trim().is_empty() {
            self.pending_username = Some(pending);
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

    /// Attach raw avatar bytes (PNG / JPEG). When present and decodable, they
    /// override the initials fallback. An eager decode check is performed here
    /// so `avatar_uses_initials_fallback()` stays accurate even before the
    /// first `show()` call. The bytes are stored for the actual
    /// GPU-upload decode in `try_paint_avatar_image`.
    pub fn with_avatar_bytes(mut self, bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            return self;
        }
        // Probe decode: just validate, discard the image data. The full decode
        // for GPU upload happens lazily in try_paint_avatar_image.
        self.avatar_decode_ok = image::load_from_memory(&bytes).is_ok();
        self.avatar_bytes = Some(bytes);
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
    /// (only meaningful in the social-profile-set variant). Returns `true`
    /// when no avatar bytes were supplied OR when the bytes failed to decode
    /// (stays honest even without a render pass).
    pub fn avatar_uses_initials_fallback(&self) -> bool {
        self.has_social_profile() && (self.avatar_bytes.is_none() || !self.avatar_decode_ok)
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
                    match (&self.dpns_handle, &self.pending_username) {
                        (Some(handle), _) => {
                            // Owned DPNS name always wins over a pending request.
                            ui.label(
                                RichText::new(format!("@{handle}"))
                                    .size(16.0)
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        }
                        (None, Some(pending)) => {
                            // Requested but not yet awarded — show the requested
                            // name with a `Pending` pill so it is not mistaken
                            // for "no username requested".
                            self.paint_pending_username_line(ui, dark_mode, pending);
                        }
                        (None, None) => {
                            // No owned name and nothing pending — show the
                            // `Pick a username` prompt so the user has a path.
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
                                pill::accent_pill(
                                    ui,
                                    self.kind.badge_label(),
                                    self.kind.badge_accent(),
                                    Some(self.kind.badge_tooltip()),
                                );
                                if let Some(label) = &self.network_label {
                                    ui.add_space(Spacing::XS);
                                    pill::accent_pill(
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

    /// Paint the avatar circle (social profile set) or the type-glyph
    /// monogram (no social profile).
    ///
    /// When `avatar_bytes` are present the raw PNG/JPEG bytes are decoded into
    /// a texture (cached in the egui context by content hash so the decode only
    /// happens once) and painted as a circle-cropped image. If decoding fails
    /// the call falls through to the initials monogram. When no bytes are set
    /// the initials or type-glyph monogram is used directly. (T09)
    fn paint_avatar_or_monogram(&self, ui: &mut Ui) {
        if let Some(bytes) = &self.avatar_bytes
            && self.try_paint_avatar_image(ui, bytes)
        {
            return;
        }
        // Social profile set → initials monogram (white on Dash-blue); else the
        // identity-type glyph on a faint tint. Shared with the breadcrumb
        // identity pill via `avatar::paint_identity_monogram`.
        let initial = self.has_social_profile().then(|| self.initials_letter());
        super::avatar::paint_identity_monogram(ui, 96.0, self.kind, initial, DashColors::DASH_BLUE);
    }

    /// Attempt to decode `bytes` (PNG / JPEG) and paint a circle-cropped avatar
    /// at the standard 96 px diameter. Returns `true` on success so the caller
    /// can skip the initials fallback.
    ///
    /// The decoded texture is cached in the egui context keyed by an FNV-1a
    /// hash of the bytes, so the `image::load_from_memory` decode only runs
    /// once per unique avatar regardless of how many frames the hero renders.
    ///
    /// # Resource note
    /// Textures accumulate in the egui temp store for the session's lifetime
    /// (one GPU allocation per distinct avatar viewed). Bounded by the number
    /// of distinct avatars seen in a session — acceptable for the typical 1-5
    /// identity case. A bounded LRU eviction policy is a follow-up if memory
    /// growth becomes a concern.
    fn try_paint_avatar_image(&self, ui: &mut Ui, bytes: &[u8]) -> bool {
        let hash = fnv1a_hash(bytes);
        let cache_id = egui::Id::new(("identity_avatar", hash));

        // Retrieve from the egui context's per-session temp store.
        let handle: Option<TextureHandle> =
            ui.ctx().data(|d| d.get_temp::<TextureHandle>(cache_id));

        let handle = match handle {
            Some(h) => h,
            None => {
                // Decode the raw bytes.
                let Ok(img) = image::load_from_memory(bytes) else {
                    return false;
                };
                let rgba = img.into_rgba8();
                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw());
                let tex =
                    ui.ctx()
                        .load_texture("identity_avatar", color_image, TextureOptions::LINEAR);
                ui.ctx().data_mut(|d| d.insert_temp(cache_id, tex.clone()));
                tex
            }
        };

        let diameter = 96.0;
        let (rect, _resp) = ui.allocate_exact_size(egui::vec2(diameter, diameter), Sense::hover());

        // Paint the image clipped to a circle via egui's `Image::corner_radius`.
        // A corner radius equal to half the diameter gives a perfect circle clip.
        // `CornerRadius::same` takes a `u8`, so clamp to 127 max (96 / 2 = 48).
        let cr = CornerRadius::same((diameter / 2.0).min(127.0) as u8);
        egui::Image::from_texture(&handle)
            .corner_radius(cr)
            .paint_at(ui, rect);

        // Overlay the same accent ring used by the initials monogram so the
        // avatar integrates visually with the rest of the card.
        let ring = Color32::from_rgba_unmultiplied(
            DashColors::DASH_BLUE.r(),
            DashColors::DASH_BLUE.g(),
            DashColors::DASH_BLUE.b(),
            51, // 20 % opacity
        );
        let stroke_w = (diameter / 48.0).max(1.0);
        ui.painter()
            .circle_stroke(rect.center(), diameter / 2.0, Stroke::new(stroke_w, ring));

        true
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

    /// Paint the pending-username line: the requested `@name` (muted, italic to
    /// signal it is not yet final) followed by a shared `Pending` pill whose
    /// tooltip carries the estimated ready time.
    fn paint_pending_username_line(&self, ui: &mut Ui, dark_mode: bool, pending: &PendingUsername) {
        let name =
            crate::model::contested_name::sanitize_pending_username_for_display(&pending.name);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("@{name}"))
                    .size(16.0)
                    .italics()
                    .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(Spacing::XS);
            pill::pending_username_pill(ui, pending);
        });
    }
}

/// FNV-1a hash of a byte slice. Used to derive a stable texture cache key for
/// avatar images so the `image::load_from_memory` decode only runs once per
/// unique byte content.
fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    }
    h
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

    // ─── Avatar readiness tests ──────────────────────────────

    /// UT-HERO-03 — avatar decode success: with_avatar_bytes() accepts valid
    /// PNG bytes and avatar_uses_initials_fallback() returns false.
    #[test]
    fn avatar_valid_bytes_disable_initials_fallback() {
        // Generate a minimal 1×1 RGBA PNG using the same `image` crate used
        // in production so the test is never out-of-sync with the decoder.
        let img = image::DynamicImage::new_rgba8(1, 1);
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .expect("writing a 1×1 RGBA PNG should always succeed");

        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.5")
            .with_display_name("Alex")
            .with_avatar_bytes(png_bytes);
        assert!(
            hero.has_social_profile(),
            "display name set → social profile variant"
        );
        assert!(
            !hero.avatar_uses_initials_fallback(),
            "valid PNG bytes → initials fallback must not fire"
        );
        assert!(
            hero.avatar_decode_ok,
            "avatar_decode_ok must be true for a valid image"
        );
    }

    /// UT-HERO-04 — avatar decode failure: with_avatar_bytes() rejects
    /// undecodable bytes and avatar_uses_initials_fallback() returns true even
    /// before any show() call (readiness flag).
    #[test]
    fn avatar_corrupt_bytes_keep_initials_fallback_honest() {
        let garbage: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xFF, 0x00];
        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.5")
            .with_display_name("Alex")
            .with_avatar_bytes(garbage);
        assert!(
            hero.has_social_profile(),
            "display name set → social profile variant"
        );
        assert!(
            hero.avatar_uses_initials_fallback(),
            "corrupt bytes must fall back to initials — readiness flag stays honest"
        );
        assert!(
            !hero.avatar_decode_ok,
            "avatar_decode_ok must be false for undecodable bytes"
        );
    }

    // ─── Pending DPNS username tests ─────────────────────────

    fn pending(name: &str) -> PendingUsername {
        PendingUsername {
            name: name.to_string(),
            decided_at: None,
        }
    }

    #[test]
    fn with_pending_username_ignores_blank_names() {
        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.0")
            .with_pending_username(pending("  "));
        assert!(hero.pending_username.is_none());
    }

    /// The pending variant renders the requested name with a `Pending` pill and
    /// suppresses the misleading `No username yet` prompt.
    #[test]
    fn hero_pending_variant_shows_pill_not_pick_username_prompt() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.0")
            .with_pending_username(pending("det1"));
        assert!(hero.dpns_handle.is_none(), "precondition: no owned name");

        let mut harness = Harness::builder().build_ui(move |ui| {
            hero.show(ui);
        });
        harness.run();

        assert!(
            harness
                .query_by_label(pill::PENDING_USERNAME_PILL_LABEL)
                .is_some(),
            "pending hero must render the 'Pending' pill"
        );
        assert!(
            harness.query_by_label("No username yet").is_none(),
            "pending hero must NOT show the 'No username yet' prompt"
        );
    }

    /// An owned DPNS name always wins: no `Pending` pill even if a pending
    /// request is also attached.
    #[test]
    fn hero_owned_handle_wins_over_pending() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let hero = IdentityHeroCard::new(HeroIdentityKind::User, "0.0")
            .with_dpns_handle("alex")
            .with_pending_username(pending("det1"));

        let mut harness = Harness::builder().build_ui(move |ui| {
            hero.show(ui);
        });
        harness.run();

        assert!(
            harness.query_by_label("@alex").is_some(),
            "owned handle must render"
        );
        assert!(
            harness
                .query_by_label(pill::PENDING_USERNAME_PILL_LABEL)
                .is_none(),
            "owned name must suppress the pending pill"
        );
    }
}
