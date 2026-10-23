//! A row of single-select chips (radio semantics).
//!
//! Each chip shows a short label and, optionally, a short detail such as a
//! balance (`Core · 0.8123`). The selected chip has a fill, a thick border and
//! a ✓ glyph, so selection never relies on colour alone. A disabled chip
//! explains itself on hover. Chips wrap onto further lines when the row is
//! narrow; in compact mode they drop the detail, which then shows on hover.
//!
//! The screen owns the selected value and feeds the chips every frame:
//!
//! ```ignore
//! let chips = self.pay_from_chips.get_or_insert_with(|| ChoiceChips::new("pay_from"));
//! chips.set_chips(vec![ChoiceChip::new(PayFrom::Auto, "Auto"), /* … */]);
//! chips.set_selected(Some(self.form.pay_from()));
//! if let Some(choice) = chips.show(ui).inner.changed_value() { /* … */ }
//! ```

use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape};
use egui::{InnerResponse, RichText, Stroke, Ui, Vec2};

/// Minimum chip size, in points (WCAG 2.5.8 target size).
const MIN_CHIP_SIZE: Vec2 = Vec2::new(24.0, 24.0);

/// One option in a [`ChoiceChips`] row.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceChip<T> {
    value: T,
    label: String,
    detail: Option<String>,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
}

impl<T> ChoiceChip<T> {
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            detail: None,
            tooltip: None,
            disabled_reason: None,
        }
    }

    /// A short detail shown after the label, e.g. a balance.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// What the option means, shown on hover.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Disable the chip; `reason` is shown on hover.
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.disabled_reason.is_none()
    }
}

/// What the user did with a [`ChoiceChips`] row this frame.
#[derive(Debug, Clone)]
pub struct ChoiceChipsResponse<T> {
    changed_value: Option<T>,
}

impl<T: Clone> ComponentResponse for ChoiceChipsResponse<T> {
    type DomainType = T;

    fn has_changed(&self) -> bool {
        self.changed_value.is_some()
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<T> {
        &self.changed_value
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// A row of single-select chips.
#[derive(Debug, Clone)]
pub struct ChoiceChips<T> {
    id_salt: String,
    chips: Vec<ChoiceChip<T>>,
    selected: Option<T>,
    compact: bool,
}

impl<T: Clone + PartialEq> ChoiceChips<T> {
    pub fn new(id_salt: impl Into<String>) -> Self {
        Self {
            id_salt: id_salt.into(),
            chips: Vec::new(),
            selected: None,
            compact: false,
        }
    }

    /// Replace the options shown.
    pub fn set_chips(&mut self, chips: Vec<ChoiceChip<T>>) {
        self.chips = chips;
    }

    /// The value shown as selected.
    pub fn set_selected(&mut self, selected: Option<T>) {
        self.selected = selected;
    }

    /// Drop the chips' details (for narrow layouts); they show on hover instead.
    pub fn set_compact(&mut self, compact: bool) {
        self.compact = compact;
    }

    fn chip_text(&self, chip: &ChoiceChip<T>, selected: bool, dark_mode: bool) -> RichText {
        let mut text = if selected {
            format!("✓ {}", chip.label)
        } else {
            chip.label.clone()
        };
        if !self.compact
            && let Some(detail) = &chip.detail
        {
            text.push_str(" · ");
            text.push_str(detail);
        }
        let color = if chip.is_enabled() {
            DashColors::text_primary(dark_mode)
        } else {
            ComponentStyles::button_disabled_text(dark_mode)
        };
        let text = RichText::new(text).color(color);
        if selected { text.strong() } else { text }
    }
}

impl<T: Clone + PartialEq> Component for ChoiceChips<T> {
    type DomainType = T;
    type Response = ChoiceChipsResponse<T>;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let dark_mode = ui.style().visuals.dark_mode;
        let mut changed_value = None;
        let response = ui
            .push_id(&self.id_salt, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    // Disabled chips carry their own readable colours, so
                    // egui's disabled fade must not dim them further.
                    ui.visuals_mut().disabled_alpha = 1.0;
                    for chip in &self.chips {
                        let selected = self.selected.as_ref() == Some(&chip.value);
                        let (fill, stroke) = if selected {
                            (
                                DashColors::DASH_BLUE.gamma_multiply(0.15),
                                Stroke::new(Shape::BORDER_WIDTH_THICK, DashColors::DASH_BLUE),
                            )
                        } else if !chip.is_enabled() {
                            (
                                ComponentStyles::button_disabled_fill(dark_mode),
                                Stroke::NONE,
                            )
                        } else {
                            (
                                DashColors::surface(dark_mode),
                                Stroke::new(
                                    Shape::BORDER_WIDTH,
                                    DashColors::border_light(dark_mode),
                                ),
                            )
                        };
                        let button = egui::Button::new(self.chip_text(chip, selected, dark_mode))
                            .selected(selected)
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(Shape::RADIUS_SM)
                            .min_size(MIN_CHIP_SIZE);
                        let mut response = ui.add_enabled(chip.is_enabled(), button);
                        let hover = match (&chip.tooltip, self.compact, &chip.detail) {
                            (Some(tooltip), true, Some(detail)) => {
                                Some(format!("{tooltip}\n{detail}"))
                            }
                            (Some(tooltip), _, _) => Some(tooltip.clone()),
                            (None, true, Some(detail)) => Some(detail.clone()),
                            (None, _, _) => None,
                        };
                        if let Some(hover) = hover {
                            response = response.clickable_tooltip(hover);
                        }
                        if let Some(reason) = &chip.disabled_reason {
                            response = response.disabled_tooltip(reason.as_str());
                        }
                        if response.clicked() && !selected {
                            changed_value = Some(chip.value.clone());
                        }
                    }
                })
                .response
            })
            .inner;
        InnerResponse::new(ChoiceChipsResponse { changed_value }, response)
    }

    fn current_value(&self) -> Option<T> {
        self.selected.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Pick {
        A,
        B,
        C,
    }

    struct State {
        chips: ChoiceChips<Pick>,
        selected: Pick,
    }

    fn harness(compact: bool) -> Harness<'static, State> {
        let mut chips = ChoiceChips::new("test_chips");
        chips.set_compact(compact);
        let state = State {
            chips,
            selected: Pick::A,
        };
        let mut harness = Harness::builder().build_ui_state(
            |ui, state: &mut State| {
                state.chips.set_chips(vec![
                    ChoiceChip::new(Pick::A, "Alpha"),
                    ChoiceChip::new(Pick::B, "Beta").with_detail("0.5000"),
                    ChoiceChip::new(Pick::C, "Gamma").disabled("Gamma has no funds."),
                ]);
                state.chips.set_selected(Some(state.selected));
                if let Some(pick) = state.chips.show(ui).inner.changed_value() {
                    state.selected = *pick;
                }
            },
            state,
        );
        harness.run();
        harness
    }

    #[test]
    fn the_selected_chip_carries_a_check_mark() {
        let harness = harness(false);
        assert!(harness.query_by_label("✓ Alpha").is_some());
        assert!(harness.query_by_label("Beta · 0.5000").is_some());
    }

    #[test]
    fn clicking_a_chip_selects_it() {
        let mut harness = harness(false);
        harness.get_by_label("Beta · 0.5000").click();
        harness.run();
        assert_eq!(harness.state().selected, Pick::B);
        assert!(harness.query_by_label("✓ Beta · 0.5000").is_some());
    }

    #[test]
    fn a_disabled_chip_cannot_be_selected() {
        let mut harness = harness(false);
        harness.get_by_label("Gamma").click();
        harness.run();
        assert_eq!(harness.state().selected, Pick::A);
    }

    /// WCAG contrast ratio of two opaque colors.
    fn contrast_ratio(a: egui::Color32, b: egui::Color32) -> f32 {
        fn luminance(color: egui::Color32) -> f32 {
            let linear = |channel: u8| {
                let value = f32::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn flatten(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| flatten(shape, out)),
            shape => out.push(shape.clone()),
        }
    }

    /// The painted glyph color of the chip labelled `label`, and the fill of
    /// the chip behind it.
    fn painted(harness: &Harness<'_, State>, label: &str) -> (egui::Color32, egui::Color32) {
        let mut shapes = Vec::new();
        for clipped in &harness.output().shapes {
            flatten(&clipped.shape, &mut shapes);
        }
        let (index, text) = shapes
            .iter()
            .enumerate()
            .find_map(|(index, shape)| match shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some((index, text)),
                _ => None,
            })
            .expect("the chip's text is painted");
        let glyph = text
            .galley
            .rows
            .iter()
            .flat_map(|row| row.visuals.mesh.vertices.iter())
            .map(|vertex| vertex.color)
            .next()
            .expect("the text has glyphs");
        // The chip's frame is the last shape under the text painted before it.
        let fill = shapes[..index]
            .iter()
            .rev()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect) if rect.rect.contains(text.pos) => Some(rect.fill),
                _ => None,
            })
            .expect("the chip's frame is painted");
        (glyph, fill)
    }

    #[test]
    fn a_disabled_chip_stays_readable_in_both_themes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let mut harness = harness(false);
            harness.ctx.set_theme(theme);
            harness.run();
            let (glyph, fill) = painted(&harness, "Gamma");
            assert_eq!(glyph.a(), 255, "{theme:?}: the label is not faded");
            assert_eq!(fill.a(), 255, "{theme:?}: the chip is not faded");
            let ratio = contrast_ratio(glyph, fill);
            assert!(
                ratio >= 4.5,
                "{theme:?}: contrast {ratio:.2} is below 4.5:1"
            );
        }
    }

    #[test]
    fn compact_chips_drop_their_detail() {
        let harness = harness(true);
        assert!(harness.query_by_label("Beta").is_some());
        assert!(harness.query_by_label("Beta · 0.5000").is_none());
    }
}
