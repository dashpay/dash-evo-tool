//! Identity pill — the third segment of the breadcrumb switcher.
//!
//! Label priority: **Local nickname → DashPay display name → DPNS username →
//! shortened Identity ID** (design-spec §G6). [`display_label`] is the one
//! resolver for that rule; every surface that names an identity goes through
//! it, so the same identity never renders two different ways.
//!
//! Follows the project's lazy-init component pattern
//! (`docs/COMPONENT_DESIGN_PATTERN.md`): domain/config fields stored on the
//! struct, the inner [`BreadcrumbPill`] built on every `show()` call.

use super::avatar::paint_identity_monogram;
use super::identity_hero_card::HeroIdentityKind;
use crate::ui::components::breadcrumb_pill::{
    BreadcrumbPill, BreadcrumbPillMode, BreadcrumbPillResponse,
};
use crate::ui::components::component_trait::ComponentResponse;
use eframe::egui::Ui;

/// Hub-wide label priority resolver for an identity. Returns the first
/// non-empty source in priority order. Never returns an empty string — an
/// empty identifier id falls back to the stable placeholder
/// `Unknown identity`. Blank and whitespace-only sources are skipped.
///
/// The arguments are listed in priority order:
///
/// * `local_nickname` — `QualifiedIdentity.alias` in the codebase, displayed
///   in the UI as "Local nickname".
/// * `display_name` — the DashPay social-profile display name. `None` on
///   surfaces that have no profile to read (e.g. the breadcrumb pill).
/// * `dpns_handle` — the identity's primary DPNS username (without the
///   leading `@`).
/// * `identity_id_base58` — the raw Base58 identity id. Shortened to
///   `"Fx1Kj…9Tt"`-style by [`shorten_id`] when used as the fallback label.
pub fn display_label(
    local_nickname: Option<&str>,
    display_name: Option<&str>,
    dpns_handle: Option<&str>,
    identity_id_base58: &str,
) -> String {
    let named = [local_nickname, display_name, dpns_handle]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty());
    if let Some(name) = named {
        return name.to_string();
    }
    let trimmed = identity_id_base58.trim();
    if trimmed.is_empty() {
        // Defensive fallback so a caller that passes an empty id does not
        // produce an invisible pill. "Unknown identity" is a complete,
        // i18n-ready sentence fragment suitable as a display label.
        return "Unknown identity".to_string();
    }
    shorten_id(trimmed)
}

/// Shorten a Base58 identity id for compact display: keeps the first five
/// and last three characters, joined by a narrow ellipsis (`…`). The raw id
/// is returned unchanged if it is too short to meaningfully shorten.
pub fn shorten_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 10 {
        return id.to_string();
    }
    let head: String = chars.iter().take(5).copied().collect();
    let tail_chars: Vec<char> = chars.iter().rev().take(3).copied().collect();
    let tail: String = tail_chars.iter().rev().copied().collect();
    format!("{head}…{tail}")
}

/// Response returned by [`IdentityPill::show`]. Thin wrapper over
/// [`BreadcrumbPillResponse`] — keeps the identity-pill API independent of
/// the underlying pill widget so callers never depend on the inner type.
#[derive(Clone, Debug)]
pub struct IdentityPillResponse {
    pub clicked: bool,
    pub label: String,
    /// The pill's inner egui `Response`, for anchoring the identity dropdown.
    /// `None` for fabricated responses (tests).
    pub response: Option<eframe::egui::Response>,
    changed_value: Option<String>,
}

impl IdentityPillResponse {
    fn from_inner(inner: BreadcrumbPillResponse) -> Self {
        let changed_value = inner.changed_value().clone();
        Self {
            clicked: inner.clicked,
            label: inner.label,
            response: inner.response,
            changed_value,
        }
    }
}

impl ComponentResponse for IdentityPillResponse {
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

/// Identity-pill widget. Stores domain/config fields (the identity's
/// nickname / handle / id) plus builder-set options. The inner
/// [`BreadcrumbPill`] is constructed on each `show()` call.
#[derive(Clone, Debug, Default)]
pub struct IdentityPill {
    local_nickname: Option<String>,
    dpns_handle: Option<String>,
    identity_id_base58: String,
    tooltip: String,
    accessible_name: Option<String>,
    mode: BreadcrumbPillMode,
    /// Optional avatar: the identity type and an optional monogram initial.
    /// When set, an 18 px circle is painted to the left and the inner pill omits
    /// the 👤 emoji icon (avoids a double avatar).
    avatar: Option<(HeroIdentityKind, Option<char>)>,
}

impl IdentityPill {
    /// Build an identity pill from the three identity fields. The pill is
    /// interactive by default; override with [`with_mode`](Self::with_mode).
    pub fn new(
        local_nickname: Option<&str>,
        dpns_handle: Option<&str>,
        identity_id_base58: &str,
    ) -> Self {
        Self {
            local_nickname: local_nickname.map(str::to_string),
            dpns_handle: dpns_handle.map(str::to_string),
            identity_id_base58: identity_id_base58.to_string(),
            tooltip: String::new(),
            accessible_name: None,
            mode: BreadcrumbPillMode::default(),
            avatar: None,
        }
    }

    /// Attach an avatar (identity type + optional monogram initial). Replaces
    /// the default 👤 emoji icon with an 18 px painted circle.
    pub fn with_avatar(mut self, kind: HeroIdentityKind, initial: Option<char>) -> Self {
        self.avatar = Some((kind, initial));
        self
    }

    /// Attach a tooltip.
    pub fn with_tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = text.into();
        self
    }

    /// Override accessible name. Defaults to the resolved display label.
    pub fn with_accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible_name = Some(name.into());
        self
    }

    /// Force a specific mode. Interactive by default.
    pub fn with_mode(mut self, mode: BreadcrumbPillMode) -> Self {
        self.mode = mode;
        self
    }

    /// Resolve the display label using the priority rule (for tests and
    /// compositional callers). The pill carries no social-profile display
    /// name, so that tier is empty here.
    pub fn resolved_label(&self) -> String {
        display_label(
            self.local_nickname.as_deref(),
            None,
            self.dpns_handle.as_deref(),
            &self.identity_id_base58,
        )
    }

    /// Build the inner `BreadcrumbPill` from the stored config. Exposed for
    /// tests; production callers use `show()`.
    fn build_inner(&self) -> BreadcrumbPill {
        let label = self.resolved_label();
        let mut pill = BreadcrumbPill::new(label).with_mode(self.mode);
        // Painted avatar replaces the emoji icon; otherwise keep the 👤 glyph.
        if self.avatar.is_none() {
            pill = pill.with_icon("👤");
        }
        if !self.tooltip.is_empty() {
            pill = pill.with_tooltip(self.tooltip.clone());
        }
        if let Some(name) = &self.accessible_name {
            pill = pill.with_accessible_name(name.clone());
        }
        pill
    }

    /// Render and return the response.
    pub fn show(&self, ui: &mut Ui) -> IdentityPillResponse {
        if let Some((kind, initial)) = self.avatar {
            let inner = ui
                .horizontal(|ui| {
                    paint_identity_monogram(ui, 18.0, kind, initial, kind.badge_accent());
                    ui.add_space(4.0);
                    self.build_inner().show(ui)
                })
                .inner;
            IdentityPillResponse::from_inner(inner)
        } else {
            IdentityPillResponse::from_inner(self.build_inner().show(ui))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nickname_wins_when_present() {
        let label = display_label(
            Some("dev"),
            Some("Alex Kim"),
            Some("alex.dash"),
            "Fx1Kj9TtFx1Kj9Tt",
        );
        assert_eq!(label, "dev");
    }

    #[test]
    fn display_name_wins_when_no_nickname() {
        let label = display_label(
            None,
            Some("Alex Kim"),
            Some("alex.dash"),
            "Fx1Kj9TtFx1Kj9Tt",
        );
        assert_eq!(label, "Alex Kim");
    }

    #[test]
    fn dpns_wins_when_no_nickname_or_display_name() {
        let label = display_label(None, None, Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn empty_nickname_falls_through_to_dpns() {
        let label = display_label(Some(""), None, Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn whitespace_nickname_falls_through_to_dpns() {
        let label = display_label(Some("   "), None, Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn blank_display_name_falls_through_to_dpns() {
        let label = display_label(None, Some("  "), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn raw_id_fallback_when_nothing_else() {
        let label = display_label(None, None, None, "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "Fx1Kj…9Tt");
    }

    #[test]
    fn short_id_not_shortened() {
        let label = display_label(None, None, None, "abcdef");
        assert_eq!(label, "abcdef");
    }

    #[test]
    fn shorten_id_handles_exactly_ten_chars() {
        assert_eq!(shorten_id("abcdefghij"), "abcdefghij");
    }

    #[test]
    fn shorten_id_shortens_long_string() {
        assert_eq!(shorten_id("abcdefghijklmn"), "abcde…lmn");
    }

    #[test]
    fn empty_id_uses_unknown_identity_fallback() {
        // Defensive fallback: an empty id must never produce an invisible
        // pill. Callers should avoid passing an empty id, but the label
        // resolver guards against it so a bug upstream never renders a blank.
        let label = display_label(None, None, None, "");
        assert_eq!(label, "Unknown identity");
    }

    #[test]
    fn whitespace_only_id_uses_unknown_identity_fallback() {
        let label = display_label(None, None, None, "   ");
        assert_eq!(label, "Unknown identity");
    }

    #[test]
    fn pill_stores_domain_fields_lazily() {
        // Lazy-init: the struct must not eagerly construct a BreadcrumbPill.
        // We verify by checking that the builder chain only mutates stored
        // config, and `build_inner` produces a pill with the right label.
        let pill = IdentityPill::new(Some("dev"), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(pill.resolved_label(), "dev");
        assert_eq!(pill.mode, BreadcrumbPillMode::Interactive);
        let inner = pill.build_inner();
        assert_eq!(inner.label(), "dev");
    }

    #[test]
    fn pill_builder_chain_is_fluent() {
        let pill = IdentityPill::new(None, Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt")
            .with_tooltip("Switch identities")
            .with_accessible_name("Identity switcher")
            .with_mode(BreadcrumbPillMode::Subdued);
        assert_eq!(pill.resolved_label(), "alex.dash");
        assert_eq!(pill.mode, BreadcrumbPillMode::Subdued);
        assert_eq!(pill.tooltip, "Switch identities");
        assert_eq!(pill.accessible_name.as_deref(), Some("Identity switcher"));
    }

    #[test]
    fn response_round_trip() {
        // Not clicked.
        let inner = BreadcrumbPillResponse::new(
            "alex.dash".to_string(),
            BreadcrumbPillMode::Interactive,
            false,
        );
        let resp = IdentityPillResponse::from_inner(inner);
        assert!(!resp.has_changed());
        assert!(resp.is_valid());
        assert!(resp.changed_value().is_none());
        assert_eq!(resp.label, "alex.dash");

        // Clicked — payload propagates.
        let inner = BreadcrumbPillResponse::new(
            "alex.dash".to_string(),
            BreadcrumbPillMode::Interactive,
            true,
        );
        let resp = IdentityPillResponse::from_inner(inner);
        assert!(resp.has_changed());
        assert_eq!(resp.changed_value().as_deref(), Some("alex.dash"));
    }
}
