//! Shared circular identity avatar / monogram painter.
//!
//! Extracted from the hero card so the hero (96 px) and the breadcrumb identity
//! pill (18 px) render the same visual. Photo rendering is deferred — like the
//! hero today, this paints an initials monogram or a type-glyph fallback.

use super::identity_hero_card::HeroIdentityKind;
use eframe::egui::{Align2, Color32, FontFamily, FontId, Response, Sense, Stroke, Ui, vec2};

/// Paint a circular identity avatar of `diameter` px at the next layout slot.
///
/// When `initial` is `Some`, fills `accent` and centres the white uppercase
/// monogram; otherwise fills a faint `accent` tint and centres the type glyph
/// for `kind`. Returns the allocated `Response` (hover-only).
pub fn paint_identity_monogram(
    ui: &mut Ui,
    diameter: f32,
    kind: HeroIdentityKind,
    initial: Option<char>,
    accent: Color32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(diameter, diameter), Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let radius = diameter * 0.5;
    let ring = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 51); // 20%
    let stroke_w = (diameter / 48.0).max(1.0); // 2px at 96, 1px at 18
    let font = FontId::new(diameter * 0.42, FontFamily::Proportional);

    match initial {
        Some(ch) => {
            painter.circle_filled(center, radius, accent);
            painter.circle_stroke(center, radius, Stroke::new(stroke_w, ring));
            painter.text(
                center,
                Align2::CENTER_CENTER,
                ch.to_string(),
                font,
                Color32::WHITE,
            );
        }
        None => {
            let tint = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 20); // 8%
            painter.circle_filled(center, radius, tint);
            painter.circle_stroke(center, radius, Stroke::new(stroke_w, ring));
            painter.text(
                center,
                Align2::CENTER_CENTER,
                kind.type_glyph(),
                font,
                accent,
            );
        }
    }
    resp
}
