//! Request card — a small card used by the Contacts tab to surface received
//! and sent contact requests. See design-spec §B.4.
//!
//! Two variants:
//! - **Received** — amber left-border, `Accept` + `Decline` buttons.
//! - **Sent** — blue left-border, `Pending` pill + `Cancel request` ghost button.
//!
//! Follows the project's lazy-init component pattern
//! (`docs/COMPONENT_DESIGN_PATTERN.md`): domain/config fields stored on the
//! struct; the inner frame is built on every `show()` call.

use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use eframe::egui::{
    Color32, CornerRadius, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2,
};

/// Copy constants, kept public so tests and sibling callsites share a single
/// source of truth. Any future i18n extraction touches one line per string.
pub const ACCEPT_LABEL: &str = "Accept";
pub const DECLINE_LABEL: &str = "Decline";
pub const PENDING_LABEL: &str = "Pending";
pub const CANCEL_LABEL: &str = "Cancel request";

/// Width of the colored strip on the left edge of each card, in logical
/// pixels. Matches the `3px` specified by the design wireframe.
pub const LEFT_BORDER_WIDTH: f32 = 3.0;

/// Which kind of request this card represents. Controls the border color,
/// button set, and accessibility metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestCardVariant {
    /// Request awaiting your approval — amber strip + Accept/Decline.
    Received,
    /// Request you've sent, waiting for the other side — blue strip + Cancel.
    Sent,
}

impl RequestCardVariant {
    /// Color of the 3px left strip. Uses the shared theme palette so dark and
    /// light modes stay consistent with the rest of the surface.
    pub fn border_color(self, dark_mode: bool) -> Color32 {
        match self {
            RequestCardVariant::Received => DashColors::warning_color(dark_mode),
            RequestCardVariant::Sent => DashColors::info_color(dark_mode),
        }
    }
}

/// Typed action from a request card. Replaces bare booleans as the
/// `ComponentResponse::DomainType` (T11). The raw booleans on
/// [`RequestCardResponse`] are kept for backward-compat call sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAction {
    /// User accepted the incoming contact request.
    Accepted,
    /// User declined the incoming contact request.
    Declined,
    /// User cancelled their outgoing contact request.
    Cancelled,
}

/// Response returned by [`RequestCard::show`]. The ad-hoc booleans
/// (`accepted`, `declined`, `cancelled`) are kept for backward-compat call
/// sites. The typed [`RequestAction`] (T11) is available via
/// [`action`](Self::action) and via the [`ComponentResponse`] impl's
/// `changed_value()`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestCardResponse {
    pub accepted: bool,
    pub declined: bool,
    pub cancelled: bool,
    /// Identifier payload attached by the caller, echoed back verbatim so the
    /// caller can route the click without re-indexing into its own state.
    pub id: Option<String>,
    /// Typed action cache populated by [`RequestCard::show`] so that
    /// `ComponentResponse::changed_value()` can return a borrow. Private —
    /// callers should use `action()` or the public booleans.
    action_cache: Option<RequestAction>,
}

impl RequestCardResponse {
    /// Derive the typed [`RequestAction`] from the ad-hoc booleans, if any.
    pub fn action(&self) -> Option<RequestAction> {
        if self.accepted {
            Some(RequestAction::Accepted)
        } else if self.declined {
            Some(RequestAction::Declined)
        } else if self.cancelled {
            Some(RequestAction::Cancelled)
        } else {
            None
        }
    }
}

impl ComponentResponse for RequestCardResponse {
    /// The typed action this frame, derived from the card's button cluster.
    type DomainType = RequestAction;

    fn has_changed(&self) -> bool {
        self.accepted || self.declined || self.cancelled
    }

    fn is_valid(&self) -> bool {
        true
    }

    /// Returns the typed action that `show()` populated into the cache.
    /// Callers using the raw booleans (`response.accepted` etc.) are
    /// unaffected — this is an additive typed view.
    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.action_cache
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// A single request card. Call [`received`](Self::received) or
/// [`sent`](Self::sent) to build — direct struct construction is
/// intentionally not public so every card has a clear variant.
#[derive(Clone, Debug)]
pub struct RequestCard {
    variant: RequestCardVariant,
    display_name: String,
    handle: String,
    /// Caller-supplied relative timestamp ("2m ago", "yesterday"). Empty string
    /// hides the timestamp slot so callers that don't have one don't need to
    /// fabricate a placeholder.
    relative_time: String,
    /// Optional identifier propagated to the response. Useful for routing
    /// clicks without maintaining a parallel index on the caller side.
    id: Option<String>,
    /// Whether this request's action is already running. Disables the action
    /// buttons, so the user is not invited to pay for a second state transition
    /// while the first is still on its way.
    busy: bool,
}

impl RequestCard {
    /// Received-variant constructor — amber strip, Accept/Decline buttons.
    pub fn received(
        display_name: impl Into<String>,
        handle: impl Into<String>,
        relative_time: impl Into<String>,
    ) -> Self {
        Self {
            variant: RequestCardVariant::Received,
            display_name: display_name.into(),
            handle: handle.into(),
            relative_time: relative_time.into(),
            id: None,
            busy: false,
        }
    }

    /// Sent-variant constructor — blue strip, Pending pill + Cancel button.
    pub fn sent(display_name: impl Into<String>, handle: impl Into<String>) -> Self {
        Self {
            variant: RequestCardVariant::Sent,
            display_name: display_name.into(),
            handle: handle.into(),
            relative_time: String::new(),
            id: None,
            busy: false,
        }
    }

    /// Attach an identifier echoed back via the response on click.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Disable the card's action buttons while its request is in flight.
    ///
    /// Accept, Decline, and Cancel each cost a real fee, so a card whose action
    /// is already running must not take another click.
    pub fn with_busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// The variant (for tests and compositional callers).
    pub fn variant(&self) -> RequestCardVariant {
        self.variant
    }

    /// Resolved border color for the current variant + theme mode.
    pub fn border_color(&self, dark_mode: bool) -> Color32 {
        self.variant.border_color(dark_mode)
    }

    /// Render the card and return its click response.
    pub fn show(&self, ui: &mut Ui) -> RequestCardResponse {
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;
        let mut response = RequestCardResponse {
            id: self.id.clone(),
            ..Default::default()
        };

        let border_color = self.border_color(dark_mode);
        let frame = Frame::new()
            .fill(DashColors::surface_elevated(dark_mode))
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border(dark_mode),
            ))
            .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
            .inner_margin(Margin {
                left: 12 + (LEFT_BORDER_WIDTH as i8),
                right: 12,
                top: 10,
                bottom: 10,
            });

        let outer = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Avatar monogram placeholder — the contact avatar lookup
                // lives in a sibling component and is wired in a follow-up.
                paint_monogram(ui, initials(&self.display_name), dark_mode);

                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&self.display_name)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format!("@{}", self.handle))
                            .small()
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    if !self.relative_time.is_empty() {
                        ui.label(
                            RichText::new(&self.relative_time)
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                });

                // Right-aligned action cluster.
                ui.with_layout(
                    eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                    |ui| {
                        let actionable = !self.busy;
                        match self.variant {
                            RequestCardVariant::Received => {
                                if ui
                                    .add_enabled(
                                        actionable,
                                        ComponentStyles::secondary_button(DECLINE_LABEL, dark_mode),
                                    )
                                    .clicked()
                                {
                                    response.declined = true;
                                }
                                ui.add_space(8.0);
                                if ui
                                    .add_enabled(
                                        actionable,
                                        ComponentStyles::primary_button(ACCEPT_LABEL),
                                    )
                                    .clicked()
                                {
                                    response.accepted = true;
                                }
                            }
                            RequestCardVariant::Sent => {
                                if ui
                                    .add_enabled(
                                        actionable,
                                        ComponentStyles::secondary_button(CANCEL_LABEL, dark_mode),
                                    )
                                    .clicked()
                                {
                                    response.cancelled = true;
                                }
                                ui.add_space(8.0);
                                paint_pending_pill(ui, dark_mode);
                            }
                        }
                    },
                );
            });
        });

        // Paint the colored 3px left strip inside the frame's allocated rect.
        let outer_rect = outer.response.rect;
        let strip_rect = Rect::from_min_max(
            Pos2::new(outer_rect.min.x + 1.0, outer_rect.min.y + 1.0),
            Pos2::new(
                outer_rect.min.x + 1.0 + LEFT_BORDER_WIDTH,
                outer_rect.max.y - 1.0,
            ),
        );
        ui.painter().rect_filled(
            strip_rect,
            CornerRadius::same(Shape::RADIUS_SM),
            border_color,
        );

        // Populate the typed action cache so ComponentResponse::changed_value()
        // can return a borrow (T11).
        response.action_cache = response.action();
        response
    }
}

/// Extract up to two uppercase initials from a display name. Falls back to
/// `?` if the name is empty or contains no word characters.
fn initials(display_name: &str) -> String {
    let mut out = String::new();
    for word in display_name.split_whitespace().take(2) {
        if let Some(c) = word.chars().find(|c| c.is_alphanumeric()) {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() { "?".to_string() } else { out }
}

/// Paint a small square monogram with the extracted initials. This is a
/// deliberately minimal placeholder — the real avatar pipeline is shared with
/// the legacy DashPay screens and will be wired in a follow-up task.
fn paint_monogram(ui: &mut Ui, initials: String, dark_mode: bool) {
    let size = Vec2::splat(36.0);
    let (rect, _response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(Shape::RADIUS_FULL),
        DashColors::surface(dark_mode),
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(Shape::RADIUS_FULL),
        Stroke::new(Shape::BORDER_WIDTH, DashColors::border(dark_mode)),
        eframe::egui::StrokeKind::Middle,
    );
    ui.painter().text(
        rect.center(),
        eframe::egui::Align2::CENTER_CENTER,
        initials,
        eframe::egui::FontId::proportional(14.0),
        DashColors::text_primary(dark_mode),
    );
}

/// Paint the small `Pending` pill used on sent request cards.
fn paint_pending_pill(ui: &mut Ui, dark_mode: bool) {
    let frame = Frame::new()
        .fill(DashColors::surface(dark_mode))
        .stroke(Stroke::new(
            Shape::BORDER_WIDTH,
            DashColors::info_color(dark_mode),
        ))
        .corner_radius(CornerRadius::same(Shape::RADIUS_FULL))
        .inner_margin(Margin::symmetric(8, 2));
    frame.show(ui, |ui| {
        ui.label(
            RichText::new(PENDING_LABEL)
                .small()
                .color(DashColors::info_color(dark_mode)),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UT-REQUEST-CARD-01 — Received vs Sent styling. The received variant
    /// resolves to the amber/warning color and carries Accept/Decline
    /// affordances; the sent variant resolves to the blue/info color and
    /// carries the Cancel/Pending affordances.
    #[test]
    fn ut_request_card_01_variant_styling() {
        let received = RequestCard::received("Alex Kim", "alex.dash", "2m ago");
        let sent = RequestCard::sent("Bao Tran", "bao.dash");

        assert_eq!(received.variant(), RequestCardVariant::Received);
        assert_eq!(sent.variant(), RequestCardVariant::Sent);

        // Received uses the amber/warning palette; sent uses the info/blue.
        for dark in [false, true] {
            assert_eq!(
                received.border_color(dark),
                DashColors::warning_color(dark),
                "received variant must use the warning/amber color"
            );
            assert_eq!(
                sent.border_color(dark),
                DashColors::info_color(dark),
                "sent variant must use the info/blue color"
            );
            assert_ne!(
                received.border_color(dark),
                sent.border_color(dark),
                "received and sent variants must be visually distinct"
            );
        }

        // Received carries Accept + Decline labels; sent carries Cancel +
        // Pending labels. Confirming via constants locks the copy down.
        assert_eq!(ACCEPT_LABEL, "Accept");
        assert_eq!(DECLINE_LABEL, "Decline");
        assert_eq!(PENDING_LABEL, "Pending");
        assert_eq!(CANCEL_LABEL, "Cancel request");
    }

    #[test]
    fn id_round_trips_through_response() {
        let card = RequestCard::received("Alex Kim", "alex.dash", "2m ago").with_id("req-123");
        // Build a fresh response the way `show` would and confirm the id is
        // propagated into the default click response.
        let response = RequestCardResponse {
            id: card.id.clone(),
            ..Default::default()
        };
        assert_eq!(response.id.as_deref(), Some("req-123"));
    }

    #[test]
    fn initials_extracts_two_uppercase_letters() {
        assert_eq!(initials("Alex Kim"), "AK");
        assert_eq!(initials("bob"), "B");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials(""), "?");
        // Non-ASCII still works — first alphanumeric wins.
        assert_eq!(initials("zoë müller"), "ZM");
    }

    #[test]
    fn received_constructor_records_timestamp() {
        let c = RequestCard::received("Alex Kim", "alex.dash", "2m ago");
        assert_eq!(c.relative_time, "2m ago");
    }

    #[test]
    fn sent_constructor_has_no_timestamp() {
        let c = RequestCard::sent("Bao Tran", "bao.dash");
        assert!(c.relative_time.is_empty());
    }

    #[test]
    fn default_response_is_all_false() {
        let r = RequestCardResponse::default();
        assert!(!r.accepted);
        assert!(!r.declined);
        assert!(!r.cancelled);
        assert!(r.id.is_none());
    }

    /// The click that reaches a card whose action is already running must not
    /// produce a second action — the second Accept would sign, broadcast, and
    /// pay for a second state transition.
    #[test]
    fn a_busy_card_does_not_act_on_a_further_click() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        use std::cell::Cell;

        for busy in [false, true] {
            let acted = Cell::new(false);
            let mut harness = Harness::builder()
                .with_size(eframe::egui::vec2(600.0, 200.0))
                .build_ui(|ui| {
                    let response = RequestCard::received("Alex Kim", "alex.dash", "2m ago")
                        .with_busy(busy)
                        .show(ui);
                    if response.action().is_some() {
                        acted.set(true);
                    }
                });

            harness.get_by_label(ACCEPT_LABEL).click();
            harness.run();

            assert_eq!(
                acted.get(),
                !busy,
                "a card with an action in flight must ignore the click, an idle one must take it"
            );
        }
    }

    #[test]
    fn action_derives_from_booleans() {
        let mut r = RequestCardResponse::default();
        assert_eq!(r.action(), None);
        r.accepted = true;
        assert_eq!(r.action(), Some(RequestAction::Accepted));
        r.accepted = false;
        r.declined = true;
        assert_eq!(r.action(), Some(RequestAction::Declined));
        r.declined = false;
        r.cancelled = true;
        assert_eq!(r.action(), Some(RequestAction::Cancelled));
    }
}
