//! Shared circular identity avatar / monogram painter.
//!
//! Extracted from the hero card so the hero (96 px) and the breadcrumb identity
//! pill (18 px) render the same visual. Photo rendering is deferred — like the
//! hero today, this paints an initials monogram or a type-glyph fallback.

use super::identity_hero_card::HeroIdentityKind;
use crate::ui::theme::DashColors;
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

/// Relative radius (fraction of the glow's max radius) and straight alpha
/// (0-255) for each concentric disc, outermost first. Layering discs from
/// largest to smallest approximates a soft radial gradient, since egui has
/// no native radial-gradient fill (same technique as the hero card's
/// diagonal gradient band, adapted from strips to circles).
const GLOW_RINGS: [(f32, u8); 4] = [(1.00, 14), (0.75, 14), (0.50, 16), (0.28, 20)];

/// Paint an abstract avatar silhouette on a soft Dash-blue radial glow.
///
/// Used for empty/onboarding states before any identity exists (design-spec
/// §B.1) — there is no identity yet to derive a monogram or type glyph from,
/// so this always renders the generic person silhouette
/// ([`HeroIdentityKind::User`]'s glyph) rather than taking a `kind` parameter.
/// `diameter` is the overall footprint (glow + glyph). Returns the allocated
/// `Response` (hover-only).
pub fn paint_abstract_avatar(ui: &mut Ui, diameter: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(diameter, diameter), Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let max_radius = diameter * 0.5;

    for (radius_frac, alpha) in GLOW_RINGS {
        let color = Color32::from_rgba_unmultiplied(
            DashColors::DASH_BLUE.r(),
            DashColors::DASH_BLUE.g(),
            DashColors::DASH_BLUE.b(),
            alpha,
        );
        painter.circle_filled(center, max_radius * radius_frac, color);
    }

    let glyph_font = FontId::new(diameter * 0.4, FontFamily::Proportional);
    painter.text(
        center,
        Align2::CENTER_CENTER,
        HeroIdentityKind::User.type_glyph(),
        glyph_font,
        DashColors::DASH_BLUE,
    );

    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    /// The glow's allocated rect must be a `diameter` x `diameter` square so
    /// callers can lay it out predictably above the onboarding heading.
    #[test]
    fn paint_abstract_avatar_allocates_square_of_requested_diameter() {
        let mut harness = Harness::builder()
            .with_size(vec2(200.0, 200.0))
            .build_ui(|ui| {
                let response = paint_abstract_avatar(ui, 140.0);
                assert_eq!(response.rect.width(), 140.0);
                assert_eq!(response.rect.height(), 140.0);
            });
        harness.run();
    }

    /// Renders in both dark and light visuals without panicking — the glow
    /// color is brand-invariant (`DashColors::DASH_BLUE`), so there is no
    /// mode-specific branch to exercise beyond "it paints".
    #[test]
    fn paint_abstract_avatar_renders_in_both_modes() {
        for dark_mode in [false, true] {
            let mut harness =
                Harness::builder()
                    .with_size(vec2(200.0, 200.0))
                    .build_ui(move |ui| {
                        ui.style_mut().visuals.dark_mode = dark_mode;
                        paint_abstract_avatar(ui, 140.0);
                    });
            harness.run();
        }
    }
}
