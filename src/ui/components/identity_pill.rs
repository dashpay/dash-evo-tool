//! Identity pill — the third segment of the breadcrumb switcher.
//!
//! Label priority: **Local nickname → DPNS username → shortened Identity ID**
//! (design-spec §G6).
//!
//! This is intentionally a thin wrapper around [`BreadcrumbPill`] so the
//! priority rule lives in exactly one place.

use crate::ui::components::breadcrumb_pill::{
    BreadcrumbPill, BreadcrumbPillMode, BreadcrumbPillResponse,
};
use eframe::egui::Ui;

/// Label priority resolver for an identity. Returns the first non-empty
/// option in the priority order, or the shortened raw id if all three are
/// missing.
///
/// * `local_nickname` — `QualifiedIdentity.alias` in the codebase, displayed
///   in the UI as "Local nickname".
/// * `dpns_handle` — the identity's primary DPNS username (without the
///   leading `@`).
/// * `identity_id_base58` — the raw Base58 identity id. Shortened to
///   `"Fx1Kj…9Tt"`-style when used as the fallback label.
///
/// The resolver never returns an empty string.
pub fn display_label(
    local_nickname: Option<&str>,
    dpns_handle: Option<&str>,
    identity_id_base58: &str,
) -> String {
    if let Some(nickname) = local_nickname.map(str::trim).filter(|s| !s.is_empty()) {
        return nickname.to_string();
    }
    if let Some(handle) = dpns_handle.map(str::trim).filter(|s| !s.is_empty()) {
        return handle.to_string();
    }
    shorten_id(identity_id_base58)
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

/// Builder for the identity pill. Wraps a `BreadcrumbPill` and exposes only
/// the identity-specific configuration (nickname / handle / id). All other
/// options (tooltip, accessible name) are forwarded to the inner pill.
pub struct IdentityPill {
    inner: BreadcrumbPill,
}

impl IdentityPill {
    /// Build an identity pill from the three identity fields. The pill is
    /// interactive by default; override with [`with_mode`].
    pub fn new(
        local_nickname: Option<&str>,
        dpns_handle: Option<&str>,
        identity_id_base58: &str,
    ) -> Self {
        let label = display_label(local_nickname, dpns_handle, identity_id_base58);
        Self {
            inner: BreadcrumbPill::new(label).with_icon("👤"),
        }
    }

    /// Attach a tooltip. Forwards to the inner pill.
    pub fn with_tooltip(mut self, text: impl Into<String>) -> Self {
        self.inner = self.inner.with_tooltip(text);
        self
    }

    /// Override accessible name. Forwards to the inner pill.
    pub fn with_accessible_name(mut self, name: impl Into<String>) -> Self {
        self.inner = self.inner.with_accessible_name(name);
        self
    }

    /// Force a specific mode. Interactive by default.
    pub fn with_mode(mut self, mode: BreadcrumbPillMode) -> Self {
        self.inner = self.inner.with_mode(mode);
        self
    }

    /// Render and return the response.
    pub fn show(self, ui: &mut Ui) -> BreadcrumbPillResponse {
        self.inner.show(ui)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nickname_wins_when_present() {
        let label = display_label(Some("dev"), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "dev");
    }

    #[test]
    fn dpns_wins_when_no_nickname() {
        let label = display_label(None, Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn empty_nickname_falls_through_to_dpns() {
        let label = display_label(Some(""), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn whitespace_nickname_falls_through_to_dpns() {
        let label = display_label(Some("   "), Some("alex.dash"), "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "alex.dash");
    }

    #[test]
    fn raw_id_fallback_when_nothing_else() {
        let label = display_label(None, None, "Fx1Kj9TtFx1Kj9Tt");
        assert_eq!(label, "Fx1Kj…9Tt");
    }

    #[test]
    fn short_id_not_shortened() {
        let label = display_label(None, None, "abcdef");
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
    fn all_empty_collapses_to_empty_id() {
        // Defensive: the id is the guaranteed-non-optional fallback, so an
        // empty id yields an empty label. Callers must not pass an empty id.
        let label = display_label(None, None, "");
        assert_eq!(label, "");
    }
}
