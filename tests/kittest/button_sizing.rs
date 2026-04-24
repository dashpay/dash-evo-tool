//! Verifies the centering fix in `ComponentStyles` helpers does not
//! cap button max width — long labels must grow, short labels must floor.

use dash_evo_tool::ui::theme::ComponentStyles;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn button_rect(harness: &mut Harness<'_>, label: &str) -> egui::Rect {
    harness.run();
    harness.get_by_label(label).rect()
}

#[test]
fn primary_button_grows_for_long_label() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = ComponentStyles::add_primary_button(ui, "Register Token Contract");
            });
        });
    let rect = button_rect(&mut harness, "Register Token Contract");
    eprintln!("Register Token Contract rect: {rect:?}");
    assert!(
        rect.width() > 120.0,
        "long label must grow past the 96px floor (actual width: {})",
        rect.width()
    );
}

#[test]
fn primary_button_floors_short_label() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = ComponentStyles::add_primary_button(ui, "OK");
            });
        });
    let rect = button_rect(&mut harness, "OK");
    assert!(
        rect.width() >= 90.0,
        "short label must still honor ~96px min width (actual: {})",
        rect.width()
    );
    assert!(
        rect.height() >= 30.0,
        "short label must honor ~36px min height (actual: {})",
        rect.height()
    );
}

#[test]
fn secondary_button_grows_for_long_label() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = ComponentStyles::add_secondary_button(
                    ui,
                    "Create Asset Lock Transaction",
                    false,
                );
            });
        });
    let rect = button_rect(&mut harness, "Create Asset Lock Transaction");
    assert!(
        rect.width() > 150.0,
        "long label on secondary must grow (actual: {})",
        rect.width()
    );
}

#[test]
fn danger_button_grows_for_long_label() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = ComponentStyles::add_danger_button(ui, "Remove From Local Database");
            });
        });
    let rect = button_rect(&mut harness, "Remove From Local Database");
    assert!(
        rect.width() > 150.0,
        "long label on danger must grow (actual: {})",
        rect.width()
    );
}

#[test]
fn primary_button_enabled_grows_for_long_label() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = ComponentStyles::add_primary_button_enabled(
                    ui,
                    true,
                    "Register Token Contract",
                );
            });
        });
    let rect = button_rect(&mut harness, "Register Token Contract");
    assert!(
        rect.width() > 120.0,
        "long label on enabled primary must grow (actual: {})",
        rect.width()
    );
}
