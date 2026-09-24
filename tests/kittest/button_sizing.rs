//! Verifies the `ComponentStyles` button helpers: labels are centered inside the
//! button, short labels honor the minimum size, and long labels keep their
//! natural width (no max-width cap, no wrapping) in both horizontal and
//! vertical layouts.

use dash_evo_tool::ui::theme::{ComponentStyles, ResponseExt};
use egui::{Align, Layout, Rect, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Parent layout the button is rendered in.
#[derive(Clone, Copy)]
enum Parent {
    Horizontal,
    /// Top-down, left-aligned — the layout that originally left-shifted short labels.
    VerticalLeft,
}

/// Rendered geometry of a single button.
struct Rendered {
    button: Rect,
    /// Painted label rects, one per text row.
    text: Vec<Rect>,
}

/// Renders one button inside `parent`, runs the harness until layout
/// stabilizes, and returns the button rect plus its painted text rects.
fn render(parent: Parent, add: impl Fn(&mut egui::Ui), label: &str) -> Rendered {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| match parent {
            Parent::Horizontal => {
                ui.horizontal(|ui| add(ui));
            }
            Parent::VerticalLeft => {
                ui.with_layout(Layout::top_down(Align::Min), |ui| add(ui));
            }
        });
    harness.run();
    let button = harness.get_by_label(label).rect();
    let mut text = Vec::new();
    for clipped in &harness.output().shapes {
        collect_text_rects(&clipped.shape, &mut text);
    }
    Rendered { button, text }
}

fn collect_text_rects(shape: &Shape, out: &mut Vec<Rect>) {
    match shape {
        Shape::Text(t) => out.push(t.visual_bounding_rect()),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_text_rects(s, out)),
        _ => {}
    }
}

fn assert_centered(r: &Rendered) {
    assert_eq!(r.text.len(), 1, "label must render as one text shape");
    let text = r.text[0];
    let dx = (text.center().x - r.button.center().x).abs();
    assert!(
        dx <= 1.5,
        "label must be horizontally centered (button {:?}, text {:?})",
        r.button,
        text
    );
}

fn assert_single_line(r: &Rendered) {
    let row_height = r.text[0].height();
    assert!(
        row_height < 24.0,
        "label must not wrap (text rect {:?})",
        r.text[0]
    );
}

fn assert_min_size(r: &Rendered) {
    let min = ComponentStyles::DIALOG_BUTTON_MIN_SIZE;
    assert!(
        r.button.width() >= min.x - 6.0 && r.button.height() >= min.y - 6.0,
        "short label must honor the ~{min:?} min size (actual: {:?})",
        r.button.size()
    );
}

#[test]
fn short_labels_are_centered_and_floored() {
    for parent in [Parent::Horizontal, Parent::VerticalLeft] {
        let r = render(
            parent,
            |ui| {
                let _ = ComponentStyles::add_primary_button(ui, "OK");
            },
            "OK",
        );
        assert_min_size(&r);
        assert_centered(&r);

        let r = render(
            parent,
            |ui| {
                let _ = ComponentStyles::add_primary_button_enabled(ui, false, "OK");
            },
            "OK",
        );
        assert_min_size(&r);
        assert_centered(&r);

        let r = render(
            parent,
            |ui| {
                let _ = ComponentStyles::add_toolbar_button(ui, "Go", egui::Color32::BLUE);
            },
            "Go",
        );
        assert_centered(&r);
    }
}

#[test]
fn long_labels_keep_natural_width_in_all_layouts() {
    type AddFn = fn(&mut egui::Ui, &str);
    let cases: [(&str, AddFn); 5] = [
        ("primary", |ui, l| {
            let _ = ComponentStyles::add_primary_button(ui, l);
        }),
        ("primary_enabled", |ui, l| {
            let _ = ComponentStyles::add_primary_button_enabled(ui, true, l);
        }),
        ("primary_disabled", |ui, l| {
            let _ = ComponentStyles::add_primary_button_enabled(ui, false, l);
        }),
        ("secondary", |ui, l| {
            let _ = ComponentStyles::add_secondary_button(ui, l, false);
        }),
        ("danger", |ui, l| {
            let _ = ComponentStyles::add_danger_button(ui, l);
        }),
    ];
    let label = "Broadcast Transition to Platform";
    for parent in [Parent::Horizontal, Parent::VerticalLeft] {
        for (name, add) in cases {
            let r = render(parent, |ui| add(ui, label), label);
            assert!(
                r.button.width() > 150.0,
                "{name}: long label must grow past the min width (actual: {})",
                r.button.width()
            );
            assert!(
                r.button.min.x >= 0.0,
                "{name}: button must not overflow to the left (rect {:?})",
                r.button
            );
            assert_single_line(&r);
            assert_centered(&r);
        }
    }
}

#[test]
fn disabled_primary_button_shows_disabled_tooltip() {
    let label = "Withdraw";
    let tip = "Please enter a valid amount to withdraw";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 300.0))
        .build_ui(|ui| {
            let _ =
                ComponentStyles::add_primary_button_enabled(ui, false, label).disabled_tooltip(tip);
        });
    harness.run();
    harness.get_by_label(label).hover();
    // Tooltips appear after a short delay; step a few frames past it.
    for _ in 0..30 {
        harness.step();
    }
    harness.run();
    assert!(
        harness.query_by_label(tip).is_some(),
        "disabled_tooltip must be visible on a styled-disabled primary button"
    );
}
