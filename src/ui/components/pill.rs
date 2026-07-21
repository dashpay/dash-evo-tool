//! Reusable inline "pill" badge — a small rounded label used for identity
//! type, network, and status indicators.
//!
//! The [`accent_pill`] renderer is the shared style: a label tinted in an
//! accent color on a 12%-accent fill with a 1px accent ring. [`pending_username_pill`]
//! builds the DPNS "still being decided" indicator on top of it so the Identity
//! Home hero card and the Identities list render an identical badge.

use crate::model::contested_name::{PendingUsername, approximate_time_until};
use crate::ui::theme::{DashColors, ResponseExt, Shape};
use eframe::egui::{
    Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, StrokeKind, Ui,
};

/// Label shown on the DPNS pending-registration pill.
pub const PENDING_USERNAME_PILL_LABEL: &str = "Pending";

/// Paint an inline pill: `label` in `accent`, on a 12%-accent fill with a 1px
/// accent ring. `tooltip`, when present, is attached on hover. Returns the
/// pill's [`Response`].
pub fn accent_pill(ui: &mut Ui, label: &str, accent: Color32, tooltip: Option<&str>) -> Response {
    let fill =
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (0.12 * 255.0) as u8);
    let stroke_color = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 180);

    let text = RichText::new(label).color(accent).size(12.0).strong();
    let inner = Frame::new()
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(Shape::RADIUS_FULL))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.add(eframe::egui::Label::new(text).sense(Sense::hover()))
        });

    // Paint the ring manually so its color matches the accent exactly.
    ui.painter().rect_stroke(
        inner.response.rect,
        CornerRadius::same(Shape::RADIUS_FULL),
        Stroke::new(1.0, stroke_color),
        StrokeKind::Outside,
    );

    match tooltip {
        Some(text) => inner.response.info_tooltip(text),
        None => inner.response,
    }
}

/// Paint the DPNS "Pending" pill for a username the identity has requested but
/// not yet been awarded. The hover tooltip carries the estimated ready time
/// when known (see [`pending_username_tooltip`]).
pub fn pending_username_pill(ui: &mut Ui, pending: &PendingUsername) -> Response {
    let tooltip = pending_username_tooltip(pending);
    accent_pill(
        ui,
        PENDING_USERNAME_PILL_LABEL,
        DashColors::WARNING_BRIGHT,
        Some(&tooltip),
    )
}

/// Build the pending pill's hover tooltip as a complete sentence. When the
/// decision time is known and still in the future, the estimate is included;
/// otherwise a generic reassurance is returned. Kept separate from the render
/// path so it is unit-testable without a frame.
pub fn pending_username_tooltip(pending: &PendingUsername) -> String {
    let now_ms = std::time::UNIX_EPOCH
        .elapsed()
        .unwrap_or_default()
        .as_millis() as u64;
    pending
        .decided_at
        .and_then(|decided_at| approximate_time_until(decided_at, now_ms))
        .unwrap_or_else(|| {
            "Dash masternodes vote on who receives this username. Check back later for updates."
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn tooltip_includes_eta_when_decision_time_is_in_the_future() {
        let now_ms = std::time::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_millis() as u64;
        let pending = PendingUsername {
            name: "det1".to_string(),
            decided_at: Some(now_ms + 3 * 3_600 * 1_000),
        };
        let tip = pending_username_tooltip(&pending);
        assert!(
            tip.contains("about 3 hours"),
            "tooltip should carry the ETA: {tip}"
        );
        assert!(tip.contains("Dash masternodes vote"));
        assert!(tip.ends_with('.'), "tooltip must be a complete sentence");
    }

    #[test]
    fn tooltip_omits_eta_when_decision_time_is_unknown_or_past() {
        for decided_at in [None, Some(0)] {
            let pending = PendingUsername {
                name: "det1".to_string(),
                decided_at,
            };
            let tip = pending_username_tooltip(&pending);
            assert!(
                !tip.contains("expected in"),
                "no ETA phrase expected: {tip}"
            );
            assert!(tip.contains("Dash masternodes vote"));
            assert!(tip.ends_with('.'), "tooltip must be a complete sentence");
        }
    }

    #[test]
    fn pending_pill_renders_the_pending_label() {
        let pending = PendingUsername {
            name: "det1".to_string(),
            decided_at: None,
        };
        let mut harness = Harness::builder().build_ui(move |ui| {
            pending_username_pill(ui, &pending);
        });
        harness.run();
        assert!(
            harness
                .query_by_label(PENDING_USERNAME_PILL_LABEL)
                .is_some(),
            "the pending pill must render its '{PENDING_USERNAME_PILL_LABEL}' label"
        );
    }
}
