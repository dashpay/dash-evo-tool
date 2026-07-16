//! Social-profile gate card — the centered card shown on the Contacts tab when
//! the current identity has no social profile. See design-spec §B.4.1.
//!
//! The card has:
//! - A heading (`Set up a social profile first.`),
//! - A body paragraph that interpolates the user's `@handle` when known,
//! - A primary button (`Add a display name`), and
//! - A secondary `Why?` button that toggles an inline explanation panel.
//!
//! Follows the project's lazy-init component pattern
//! (`docs/COMPONENT_DESIGN_PATTERN.md`): domain/config fields stored on the
//! struct; visual primitives are built on every `show()` call.

use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::theme::{ComponentStyles, DashColors, Shadow, Shape};
use eframe::egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};

/// Copy constants, kept as `pub const` so tests and sibling callsites share a
/// single source of truth and any future i18n extraction touches one line.
pub const HEADING: &str = "Set up a social profile first.";
pub const PRIMARY_LABEL: &str = "Add a display name";
pub const WHY_LABEL: &str = "Why?";
pub const WHY_EXPANDED_LABEL: &str = "Hide details";

/// Body text when the caller knows the identity's DPNS handle. The `{handle}`
/// placeholder is i18n-ready (named, no positional assumptions).
pub const BODY_WITH_HANDLE: &str = "Contacts use your display name and avatar to let people find you. Your username \
     @{handle} already works for payments — a social profile only unlocks contacts. \
     Without a social profile, you cannot add contacts or receive contact requests.";

/// Fallback body used when no DPNS handle is available yet. Avoids emitting a
/// stray `@` placeholder that would confuse everyday users.
pub const BODY_NO_HANDLE: &str = "Contacts use your display name and avatar to let people find you. A social profile \
     only unlocks contacts. Without a social profile, you cannot add contacts or \
     receive contact requests.";

/// Text shown inside the expanded "Why?" panel.
pub const WHY_PANEL_BODY: &str = "Your username is already yours — it's on Platform and anyone who knows it can pay \
     you. A social profile is different: it adds a display name and avatar, and unlocks \
     the contacts feature so your friends can find you, send you money by name, and see \
     your recent activity if you allow it.";

/// Interpolate `{handle}` into a template. Public for unit tests. Missing
/// placeholder returns the template unchanged — callers that pass a template
/// without a placeholder still get a sensible output.
pub(crate) fn interpolate_handle(template: &str, handle: &str) -> String {
    template.replace("{handle}", handle)
}

/// Typed action emitted by [`SocialProfileGateCard`] (T12). Replaces the
/// meaning of the raw booleans with an explicit discriminant while keeping the
/// booleans for backward-compat call sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateCardAction {
    /// The primary CTA ("Add a display name") was clicked.
    PrimaryClicked,
    /// The "Why?" / "Hide details" toggle was clicked.
    WhyToggled,
}

/// Response returned by [`SocialProfileGateCard::show`]. The raw booleans
/// (`primary_clicked`, `why_toggled`) are kept for backward-compat call sites.
/// The typed [`GateCardAction`] (T12) is available via
/// [`action`](Self::action) and via the [`ComponentResponse`] impl.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialProfileGateCardResponse {
    /// The primary button was clicked this frame.
    pub primary_clicked: bool,
    /// The `Why?` toggle was clicked this frame.
    pub why_toggled: bool,
    /// Typed action cache populated by [`SocialProfileGateCard::show`] so that
    /// `ComponentResponse::changed_value()` can return a borrow. Private.
    action_cache: Option<GateCardAction>,
}

impl SocialProfileGateCardResponse {
    /// Derive the typed [`GateCardAction`] from the raw booleans, if any.
    pub fn action(&self) -> Option<GateCardAction> {
        if self.primary_clicked {
            Some(GateCardAction::PrimaryClicked)
        } else if self.why_toggled {
            Some(GateCardAction::WhyToggled)
        } else {
            None
        }
    }
}

impl ComponentResponse for SocialProfileGateCardResponse {
    /// The typed action from this frame's interaction with the gate card.
    type DomainType = GateCardAction;

    fn has_changed(&self) -> bool {
        self.primary_clicked || self.why_toggled
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.action_cache
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// The centered no-profile gate card. Interactive-only state is the `expanded`
/// flag for the explanation panel, which the caller owns so the card can stay
/// stateless between frames and is trivial to test.
#[derive(Clone, Debug, Default)]
pub struct SocialProfileGateCard {
    handle: Option<String>,
    expanded: bool,
    max_width: Option<f32>,
}

impl SocialProfileGateCard {
    /// Construct a new gate card. Handle is optional — when absent, the card
    /// falls back to the no-handle body copy so users in the pre-DPNS state
    /// never see a stray `@{handle}` placeholder.
    pub fn new(handle: Option<&str>) -> Self {
        Self {
            handle: handle
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            expanded: false,
            max_width: None,
        }
    }

    /// Set whether the "Why?" explanation panel is expanded. Caller-owned so
    /// the card has no cross-frame mutable state.
    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Clamp the card body width. Defaults to a reasonable reading width.
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.max_width = Some(max_width);
        self
    }

    /// Resolved body text — handle-aware when a handle is set.
    pub fn resolved_body(&self) -> String {
        match &self.handle {
            Some(h) => interpolate_handle(BODY_WITH_HANDLE, h),
            None => BODY_NO_HANDLE.to_string(),
        }
    }

    /// Resolved secondary-button label: flips between "Why?" and a collapse
    /// affordance when the panel is expanded. Exposed for tests.
    pub fn resolved_why_label(&self) -> &'static str {
        if self.expanded {
            WHY_EXPANDED_LABEL
        } else {
            WHY_LABEL
        }
    }

    /// Render the card. Returns a response describing which buttons were
    /// clicked this frame; the caller owns the expansion state.
    pub fn show(&self, ui: &mut Ui) -> SocialProfileGateCardResponse {
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;
        let max_width = self.max_width.unwrap_or(520.0);
        let mut response = SocialProfileGateCardResponse::default();

        ui.vertical_centered(|ui| {
            ui.set_max_width(max_width);
            ui.add_space(24.0);

            let frame = Frame::new()
                .fill(DashColors::surface_elevated(dark_mode))
                .stroke(Stroke::new(
                    Shape::BORDER_WIDTH,
                    DashColors::border(dark_mode),
                ))
                .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
                .shadow(Shadow::medium())
                .inner_margin(Margin::symmetric(24, 24));

            frame.show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(HEADING)
                            .strong()
                            .size(20.0)
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(self.resolved_body())
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        // Spacer to center-align the two buttons inside the
                        // vertical_centered column.
                        ui.add_space(0.0);
                        if ui
                            .add(ComponentStyles::primary_button(PRIMARY_LABEL))
                            .clicked()
                        {
                            response.primary_clicked = true;
                        }
                        ui.add_space(8.0);
                        if ui
                            .add(ComponentStyles::secondary_button(
                                self.resolved_why_label(),
                                dark_mode,
                            ))
                            .clicked()
                        {
                            response.why_toggled = true;
                        }
                    });
                    if self.expanded {
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(WHY_PANEL_BODY)
                                .color(DashColors::text_secondary(dark_mode))
                                .small(),
                        );
                    }
                });
            });

            ui.add_space(24.0);
        });

        // Populate the typed action cache so ComponentResponse::changed_value()
        // can return a borrow (T12).
        response.action_cache = response.action();
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UT-GATE-01 — No-social-profile gate card: interpolates `{handle}`
    /// correctly and the primary button label is `Add a display name`.
    #[test]
    fn ut_gate_01_interpolates_handle_and_primary_label() {
        let card = SocialProfileGateCard::new(Some("alex.dash"));
        let body = card.resolved_body();
        assert!(
            body.contains("@alex.dash"),
            "body must interpolate the handle with a leading @, got: {body}"
        );
        assert!(
            !body.contains("{handle}"),
            "body must not contain the raw placeholder after interpolation"
        );
        assert_eq!(PRIMARY_LABEL, "Add a display name");
    }

    #[test]
    fn interpolate_handle_replaces_placeholder() {
        assert_eq!(
            interpolate_handle("Hello @{handle}, welcome", "bob"),
            "Hello @bob, welcome"
        );
    }

    #[test]
    fn interpolate_handle_leaves_template_without_placeholder_unchanged() {
        assert_eq!(
            interpolate_handle("No placeholder here.", "bob"),
            "No placeholder here."
        );
    }

    #[test]
    fn missing_handle_falls_back_to_no_handle_body() {
        let card = SocialProfileGateCard::new(None);
        let body = card.resolved_body();
        assert_eq!(body, BODY_NO_HANDLE);
        assert!(
            !body.contains("{handle}"),
            "no-handle body must never contain a raw placeholder"
        );
        assert!(
            !body.contains('@'),
            "no-handle body must not contain a stray @ marker"
        );
    }

    #[test]
    fn empty_or_whitespace_handle_treated_as_absent() {
        assert!(SocialProfileGateCard::new(Some("")).handle.is_none());
        assert!(SocialProfileGateCard::new(Some("   ")).handle.is_none());
    }

    #[test]
    fn why_label_flips_when_expanded() {
        let collapsed = SocialProfileGateCard::new(None);
        let expanded = SocialProfileGateCard::new(None).with_expanded(true);
        assert_eq!(collapsed.resolved_why_label(), WHY_LABEL);
        assert_eq!(expanded.resolved_why_label(), WHY_EXPANDED_LABEL);
    }

    #[test]
    fn heading_is_complete_sentence() {
        assert!(HEADING.ends_with('.'));
        assert!(HEADING.chars().next().unwrap().is_ascii_uppercase());
    }

    #[test]
    fn default_response_has_no_clicks() {
        let resp = SocialProfileGateCardResponse::default();
        assert!(!resp.primary_clicked);
        assert!(!resp.why_toggled);
    }
}
