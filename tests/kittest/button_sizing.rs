//! Verifies the centering fix in `ComponentStyles` helpers does not
//! cap button max width — long labels must grow, short labels must floor.

use dash_evo_tool::ui::theme::ComponentStyles;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Builds a single-button harness inside a horizontal layout, runs one frame,
/// and returns the rendered button's rect. `add` may run more than once, since
/// `Harness` re-invokes the UI closure until layout stabilizes.
fn button_rect_for(add: impl Fn(&mut egui::Ui), label: &str) -> egui::Rect {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| {
            ui.horizontal(|ui| add(ui));
        });
    harness.run();
    harness.get_by_label(label).rect()
}

#[test]
fn primary_button_grows_for_long_label() {
    let label = "Register Token Contract";
    let rect = button_rect_for(
        |ui| {
            let _ = ComponentStyles::add_primary_button(ui, label);
        },
        label,
    );
    assert!(
        rect.width() > 120.0,
        "long label must grow past the 96px floor (actual width: {})",
        rect.width()
    );
}

#[test]
fn primary_button_floors_short_label() {
    let rect = button_rect_for(
        |ui| {
            let _ = ComponentStyles::add_primary_button(ui, "OK");
        },
        "OK",
    );
    assert!(
        rect.width() >= ComponentStyles::DIALOG_BUTTON_MIN_SIZE.x - 6.0,
        "short label must still honor the ~{}px min width (actual: {})",
        ComponentStyles::DIALOG_BUTTON_MIN_SIZE.x,
        rect.width()
    );
    assert!(
        rect.height() >= ComponentStyles::DIALOG_BUTTON_MIN_SIZE.y - 6.0,
        "short label must honor the ~{}px min height (actual: {})",
        ComponentStyles::DIALOG_BUTTON_MIN_SIZE.y,
        rect.height()
    );
}

#[test]
fn secondary_button_grows_for_long_label() {
    let label = "Create Asset Lock Transaction";
    let rect = button_rect_for(
        |ui| {
            let _ = ComponentStyles::add_secondary_button(ui, label, false);
        },
        label,
    );
    assert!(
        rect.width() > 150.0,
        "long label on secondary must grow (actual: {})",
        rect.width()
    );
}

#[test]
fn danger_button_grows_for_long_label() {
    let label = "Remove From Local Database";
    let rect = button_rect_for(
        |ui| {
            let _ = ComponentStyles::add_danger_button(ui, label);
        },
        label,
    );
    assert!(
        rect.width() > 150.0,
        "long label on danger must grow (actual: {})",
        rect.width()
    );
}

#[test]
fn primary_button_enabled_grows_for_long_label() {
    let label = "Register Token Contract";
    let rect = button_rect_for(
        |ui| {
            let _ = ComponentStyles::add_primary_button_enabled(ui, true, label);
        },
        label,
    );
    assert!(
        rect.width() > 120.0,
        "long label on enabled primary must grow (actual: {})",
        rect.width()
    );
}
