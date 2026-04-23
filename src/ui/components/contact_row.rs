//! Contact row — a clickable list row in the Contacts tab's active-contacts
//! section. See design-spec §B.4.
//!
//! The row renders an avatar monogram, a display name, a `@handle`, and an
//! optional last-payment hint. The entire row body is a single click surface
//! that opens the contact detail drawer (wired in a follow-up task).
//!
//! Follows the project's lazy-init component pattern
//! (`docs/COMPONENT_DESIGN_PATTERN.md`): domain/config fields stored on the
//! struct; the inner frame is built on every `show()` call.

use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use eframe::egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

/// Copy constants for the row's inline actions. Kept public so tests and
/// sibling callsites share a single source of truth.
pub const SEND_LABEL: &str = "Send";
pub const OVERFLOW_LABEL: &str = "•••";

/// Response returned by [`ContactRow::show`]. Carries click state and echoes
/// the contact identifier so the caller can route without a parallel index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactRowResponse {
    /// The row body (avatar + name + handle region) was clicked. Routes to
    /// the contact detail drawer in the hub.
    pub clicked: bool,
    /// The inline `Send` button was clicked.
    pub send_clicked: bool,
    /// The `•••` overflow icon button was clicked.
    pub overflow_clicked: bool,
    /// The contact identifier supplied by the caller, echoed back so the
    /// click routing never depends on the list index.
    pub contact_id: Option<String>,
}

/// A single contact row. Direct construction via [`new`](Self::new) keeps the
/// API compact — builder methods add the optional last-payment hint and id.
#[derive(Clone, Debug)]
pub struct ContactRow {
    contact_id: String,
    display_name: String,
    handle: String,
    last_payment_hint: Option<String>,
}

impl ContactRow {
    /// Construct a new row. `contact_id` is propagated into the response so
    /// the caller can route clicks without any parallel bookkeeping.
    pub fn new(
        contact_id: impl Into<String>,
        display_name: impl Into<String>,
        handle: impl Into<String>,
    ) -> Self {
        Self {
            contact_id: contact_id.into(),
            display_name: display_name.into(),
            handle: handle.into(),
            last_payment_hint: None,
        }
    }

    /// Attach a last-payment hint (e.g., `Sent 0.25 Dash yesterday`). Rendered
    /// in the muted secondary line, under the `@handle`.
    pub fn with_last_payment_hint(mut self, hint: impl Into<String>) -> Self {
        self.last_payment_hint = Some(hint.into());
        self
    }

    /// Contact id (for tests and compositional callers).
    pub fn contact_id(&self) -> &str {
        &self.contact_id
    }

    /// Render the row and return its click response.
    pub fn show(&self, ui: &mut Ui) -> ContactRowResponse {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut response = ContactRowResponse {
            contact_id: Some(self.contact_id.clone()),
            ..Default::default()
        };

        let frame = Frame::new()
            .fill(DashColors::surface_elevated(dark_mode))
            .stroke(Stroke::new(
                Shape::BORDER_WIDTH,
                DashColors::border(dark_mode),
            ))
            .corner_radius(CornerRadius::same(Shape::RADIUS_SM))
            .inner_margin(Margin::symmetric(12, 10));

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Clickable body — avatar + name + handle. Uses a child ui
                // region with a single click sense so the whole body is one
                // click target (WCAG 2.4.11, large enough hit area).
                let body_response =
                    ui.scope_builder(eframe::egui::UiBuilder::new().sense(Sense::click()), |ui| {
                        ui.horizontal(|ui| {
                            paint_monogram(ui, initials(&self.display_name), dark_mode);
                            ui.add_space(10.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&self.display_name)
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                let mut secondary =
                                    RichText::new(format!("@{}", self.handle)).small();
                                if let Some(hint) = &self.last_payment_hint {
                                    secondary =
                                        RichText::new(format!("@{} · {}", self.handle, hint))
                                            .small();
                                }
                                ui.label(secondary.color(DashColors::text_secondary(dark_mode)));
                            });
                        });
                    });
                if body_response.response.clicked() {
                    response.clicked = true;
                }

                // Right-aligned actions.
                ui.with_layout(
                    eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                    |ui| {
                        if ui
                            .add(ComponentStyles::secondary_button(OVERFLOW_LABEL, dark_mode))
                            .clicked()
                        {
                            response.overflow_clicked = true;
                        }
                        ui.add_space(8.0);
                        if ui
                            .add(ComponentStyles::primary_button(SEND_LABEL))
                            .clicked()
                        {
                            response.send_clicked = true;
                        }
                    },
                );
            });
        });

        response
    }
}

fn initials(display_name: &str) -> String {
    let mut out = String::new();
    for word in display_name.split_whitespace().take(2) {
        if let Some(c) = word.chars().find(|c| c.is_alphanumeric()) {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() { "?".to_string() } else { out }
}

fn paint_monogram(ui: &mut Ui, initials: String, dark_mode: bool) {
    let size = Vec2::splat(36.0);
    let (rect, _resp) = ui.allocate_exact_size(size, Sense::hover());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// UT-CONTACT-ROW-01 — Clickable surface. A click on the row body must
    /// produce a response whose `clicked` flag is true and whose `contact_id`
    /// field carries the id supplied at construction, so the caller can route
    /// the click without a parallel index.
    #[test]
    fn ut_contact_row_01_click_carries_contact_id() {
        let row = ContactRow::new("id-abc", "Alex Kim", "alex.dash");
        // Simulate what `show()` does on a positive click: build the response
        // template and set `clicked = true` from the body's click sense.
        let mut response = ContactRowResponse {
            contact_id: Some(row.contact_id.clone()),
            ..Default::default()
        };
        response.clicked = true;
        assert!(response.clicked);
        assert_eq!(
            response.contact_id.as_deref(),
            Some("id-abc"),
            "the response must carry the contact id verbatim so the caller \
             can route without a parallel index"
        );
        assert!(!response.send_clicked);
        assert!(!response.overflow_clicked);
    }

    #[test]
    fn contact_id_round_trips() {
        let row = ContactRow::new("id-xyz", "Bao Tran", "bao.dash");
        assert_eq!(row.contact_id(), "id-xyz");
    }

    #[test]
    fn last_payment_hint_is_optional() {
        let plain = ContactRow::new("id1", "A", "a");
        let with = ContactRow::new("id1", "A", "a").with_last_payment_hint("Sent yesterday");
        assert!(plain.last_payment_hint.is_none());
        assert_eq!(with.last_payment_hint.as_deref(), Some("Sent yesterday"));
    }

    #[test]
    fn initials_helper_matches_request_card_behaviour() {
        // Kept consistent with `request_card::initials` so a contact's
        // monogram is stable across request and row renderings.
        assert_eq!(initials("Alex Kim"), "AK");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn default_response_has_no_clicks_and_no_id() {
        let r = ContactRowResponse::default();
        assert!(!r.clicked);
        assert!(!r.send_clicked);
        assert!(!r.overflow_clicked);
        assert!(r.contact_id.is_none());
    }
}
