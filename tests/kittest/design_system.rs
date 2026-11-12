use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::theme::{DashColors, ThemeMode, apply_theme};
use egui::{Color32, Shape};
use egui_kittest::kittest::Queryable;

fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(color: Color32) -> f32 {
        let linear = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn text_colors(shape: &Shape, label: &str, colors: &mut Vec<Color32>) {
    match shape {
        Shape::Text(text) if text.galley.job.text == label => {
            for section in &text.galley.job.sections {
                colors.push(text.override_text_color.unwrap_or_else(|| {
                    if section.format.color == Color32::PLACEHOLDER {
                        text.fallback_color
                    } else {
                        section.format.color
                    }
                }));
            }
        }
        Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| text_colors(shape, label, colors)),
        _ => {}
    }
}

#[test]
fn design_system_wallet_navigation_text_is_readable_in_both_themes() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
            apply_theme(&harness.ctx, mode);
            harness.run_steps(3);
            let mut colors = Vec::new();
            for shape in &harness.output().shapes {
                text_colors(&shape.shape, "Wallets", &mut colors);
            }
            assert!(!colors.is_empty(), "Wallet navigation must actually render");
            for color in colors {
                assert!(
                    contrast(color, DashColors::background(dark)) >= 4.5,
                    "{mode:?}: actual Wallets navigation text {color:?} must contrast with its surface"
                );
            }
        }
    });
}

#[test]
fn design_system_creator_fields_and_navigation_fit_small_windows() {
    for size in [egui::vec2(1260.0, 780.0), egui::vec2(800.0, 600.0)] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            with_isolated_data_dir(|| {
                let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
                harness.set_size(size);
                apply_theme(&harness.ctx, mode);
                harness.run_steps(5);
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                harness
                    .get_by_role_and_label(egui::accesskit::Role::TextInput, "Token name")
                    .scroll_to_me();
                harness.run_steps(5);
                let field = harness
                    .get_by_role_and_label(egui::accesskit::Role::TextInput, "Token name")
                    .rect();
                let footer = harness.get_by_label_contains("Continue:").rect();
                let editor_clip = harness
                    .output()
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        let mut colors = Vec::new();
                        text_colors(&shape.shape, "Token name", &mut colors);
                        (!colors.is_empty()).then_some(shape.clip_rect)
                    })
                    .expect("The scrolled field label must render");
                assert!(
                    editor_clip.height() >= 144.0,
                    "{size:?}/{mode:?}: editor must show at least two labelled inputs: {editor_clip:?}"
                );
                assert!(
                    field.height() >= 43.0 && field.width() >= 120.0,
                    "{size:?}/{mode:?}: {field:?}"
                );
                assert!(
                    viewport.contains_rect(field) && field.bottom() <= footer.top(),
                    "{size:?}/{mode:?}: scrolled field {field:?}, footer {footer:?}"
                );
                for section in 1..=6 {
                    if section == 3 {
                        harness
                            .get_by_label("Enable recurring distribution")
                            .scroll_to_me();
                        harness.run_steps(5);
                        harness
                            .get_by_label("Enable recurring distribution")
                            .click();
                        harness.run_steps(5);
                        harness
                            .get_by_role_and_label(egui::accesskit::Role::TextInput, "Repeat every")
                            .scroll_to_me();
                        harness.run_steps(5);
                        let field = harness
                            .get_by_role_and_label(egui::accesskit::Role::TextInput, "Repeat every")
                            .rect();
                        assert!(
                            viewport.contains_rect(field),
                            "{size:?}/{mode:?}: recurring field {field:?}"
                        );
                    }
                    if section == 4 {
                        harness.get_by_label("Add group").scroll_to_me();
                        harness.run_steps(5);
                        harness.get_by_label("Add group").click();
                        harness.run_steps(5);
                        harness
                            .get_by_role_and_label(
                                egui::accesskit::Role::TextInput,
                                "Group name (local only)",
                            )
                            .scroll_to_me();
                        harness.run_steps(5);
                        let field = harness
                            .get_by_role_and_label(
                                egui::accesskit::Role::TextInput,
                                "Group name (local only)",
                            )
                            .rect();
                        assert!(
                            viewport.contains_rect(field),
                            "{size:?}/{mode:?}: group field {field:?}"
                        );
                    }
                    let next = harness.get_by_label_contains("Continue:");
                    assert!(
                        viewport.contains_rect(next.rect()),
                        "{size:?}/{mode:?}: section {section} next action {:?}",
                        next.rect()
                    );
                    next.click();
                    harness.run_steps(5);
                    let back = harness.get_by_label("← Back").rect();
                    assert!(
                        viewport.contains_rect(back),
                        "{size:?}/{mode:?}: section {} back action {back:?}",
                        section + 1
                    );
                }
                harness.get_by_label("Review your token");
            });
        }
    }
}
