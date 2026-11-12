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

fn rendered_fonts(shape: &Shape, label: &str, fonts: &mut Vec<egui::FontId>) {
    match shape {
        Shape::Text(text) if text.galley.job.text == label => {
            fonts.extend(
                text.galley
                    .job
                    .sections
                    .iter()
                    .map(|section| section.format.font_id.clone()),
            );
        }
        Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| rendered_fonts(shape, label, fonts)),
        _ => {}
    }
}

#[test]
fn design_system_creator_summary_has_subordinate_heading_and_prominent_name() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
        harness.set_size(egui::vec2(1260.0, 900.0));
        for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
            apply_theme(&harness.ctx, mode);
            harness.run_steps(5);
            for (label, size) in [
                ("What are you creating?", 21.0),
                ("Your token", 16.0),
                ("Untitled token", 19.0),
            ] {
                let mut fonts = Vec::new();
                for shape in &harness.output().shapes {
                    rendered_fonts(&shape.shape, label, &mut fonts);
                }
                assert!(!fonts.is_empty(), "{label} must actually render");
                for font in fonts {
                    assert_eq!(font.size, size, "{mode:?}: {label}");
                    assert_eq!(
                        font.family,
                        egui::FontFamily::Name("noto_sans_semibold".into()),
                        "{mode:?}: {label} needs a real strong face"
                    );
                }
            }
            let label = harness.get_by_label("Initial supply").rect();
            let mut values = harness.get_all_by_label("not valid yet");
            assert!(
                values.any(|value| value.rect().top() > label.bottom()
                    && (value.rect().left() - label.left()).abs() <= 1.0),
                "The supply value must sit below its label"
            );
            let mut colors = Vec::new();
            let mut fonts = Vec::new();
            for shape in &harness.output().shapes {
                text_colors(&shape.shape, "Initial supply", &mut colors);
                rendered_fonts(&shape.shape, "Initial supply", &mut fonts);
            }
            assert!(!colors.is_empty());
            assert!(
                colors
                    .iter()
                    .all(|color| *color == DashColors::text_secondary(dark))
            );
            assert!(fonts.iter().all(|font| font.size == 12.0));
            let mut value_fonts = Vec::new();
            for shape in &harness.output().shapes {
                rendered_fonts(&shape.shape, "not valid yet", &mut value_fonts);
            }
            assert!(!value_fonts.is_empty());
            assert!(value_fonts.iter().all(|font| font.size == 13.0));
        }
    });
}

#[test]
fn design_system_creator_label_rows_do_not_inherit_button_height() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
        harness.set_size(egui::vec2(1260.0, 900.0));
        harness.run_steps(5);
        let help = harness.get_by_label("Help: Token name").rect();
        let field = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, "Token name")
            .rect();
        assert!(
            field.top() - help.bottom() <= 8.1,
            "The 28-point label/help row must not reserve a 40-point button row: {help:?} / {field:?}"
        );
        assert!(field.height() >= 43.0);
        assert!(harness.get_by_label_contains("Continue:").rect().height() >= 40.0);
    });
}

#[test]
fn design_system_settings_available_status_has_readable_foreground() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenNetworkChooser);
        harness.set_size(egui::vec2(1260.0, 900.0));
        let app_context = harness.state().current_app_context().clone();
        for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
            apply_theme(&harness.ctx, mode);
            app_context.connection_status().set_dapi_status(3, 3);
            harness.run_steps(3);
            let label = app_context.connection_status().dapi_status_label();
            harness.get_by_label(&label).scroll_to_me();
            harness.run_steps(3);
            let mut colors = Vec::new();
            for shape in &harness.output().shapes {
                text_colors(&shape.shape, &label, &mut colors);
            }
            assert!(
                !colors.is_empty(),
                "Available DAPI status must actually render"
            );
            for color in colors {
                assert!(
                    contrast(color, DashColors::surface(dark)) >= 4.5,
                    "{mode:?}: actual available status {color:?} needs readable contrast"
                );
            }
        }
    });
}

#[test]
fn design_system_network_toolbar_text_contrasts_in_both_themes() {
    use dash_evo_tool::ui::theme::ComponentStyles;
    use dash_sdk::dashcore_rpc::dashcore::Network;
    for network in [
        Network::Mainnet,
        Network::Testnet,
        Network::Devnet,
        Network::Regtest,
    ] {
        for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
            let fill = DashColors::network_accent(network, dark);
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(500.0, 150.0))
                .build_ui(|ui| {
                    ComponentStyles::add_toolbar_button(ui, "Action", fill);
                    ComponentStyles::add_toolbar_menu_button(ui, "Menu", fill);
                });
            apply_theme(&harness.ctx, mode);
            harness.run_steps(3);
            for label in ["Action", "Menu ▾"] {
                let mut colors = Vec::new();
                for shape in &harness.output().shapes {
                    text_colors(&shape.shape, label, &mut colors);
                }
                assert!(!colors.is_empty(), "{label} must actually render");
                for color in colors {
                    assert!(
                        contrast(color, fill) >= 4.5,
                        "{network:?}/{mode:?}: {label} foreground {color:?} on {fill:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn design_system_creator_explanations_use_muted_roles_in_every_section() {
    let samples = [
        [
            (
                "Give people a recognizable name, then choose how the supply should work.",
                14.0,
            ),
            ("Required. For example, StudioCredit.", 12.0),
        ],
        [
            (
                "Enter amounts in tokens or base units. Changing precision preserves the displayed token amount.",
                14.0,
            ),
            (
                "Burning tokens can free capacity for future mints; this is not a lifetime minting limit.",
                12.0,
            ),
        ],
        [
            (
                "Three independent ways to distribute tokens. Turning one off keeps its settings in your draft.",
                14.0,
            ),
            (
                "Allocate tokens at repeated time, epoch or block intervals. Recipients must claim eligible allocations.",
                12.0,
            ),
        ],
        [
            (
                "Share authority using weighted approvals. A group acts when its approving members meet the required power.",
                14.0,
            ),
            (
                "Labels are local. Review shows the positions stored in the contract.",
                12.0,
            ),
        ],
        [
            (
                "Choose which token events are retained. These choices are independent from the powers that perform the actions. They configure the contract's event histories, not local privacy settings.",
                14.0,
            ),
            (
                "This does not hide the contract or make transactions private.",
                12.0,
            ),
        ],
        [
            (
                "Help people recognize the token, and optionally define related document types.",
                14.0,
            ),
            (
                "Explain the token's purpose in plain language. Use 3 to 100 UTF-8 bytes when provided.",
                12.0,
            ),
        ],
        [
            ("Check the choices your holders will live with.", 14.0),
            (
                "Check the settings and owner without spending credits. You will confirm creation separately. Editing the draft requires another check.",
                14.0,
            ),
        ],
    ];
    let mut failures = Vec::new();
    for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
        with_isolated_data_dir(|| {
            let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
            harness.set_size(egui::vec2(1260.0, 900.0));
            apply_theme(&harness.ctx, mode);
            harness.run_steps(5);
            for (section, labels) in samples.iter().enumerate() {
                for (label, size) in labels {
                    harness.get_by_label(label).scroll_to_me();
                    harness.run_steps(5);
                    let mut colors = Vec::new();
                    let mut fonts = Vec::new();
                    for shape in &harness.output().shapes {
                        text_colors(&shape.shape, label, &mut colors);
                        rendered_fonts(&shape.shape, label, &mut fonts);
                    }
                    if colors.is_empty()
                        || colors
                            .iter()
                            .any(|color| *color != DashColors::text_secondary(dark))
                        || fonts.is_empty()
                        || fonts.iter().any(|font| font.size != *size)
                    {
                        failures.push(format!(
                            "{mode:?}/{section}: {label}: {colors:?}, {fonts:?}"
                        ));
                    }
                }
                if section < samples.len() - 1 {
                    harness.get_by_label_contains("Continue:").click();
                    harness.run_steps(5);
                }
            }
        });
    }
    assert!(
        failures.is_empty(),
        "Muted explanatory roles:\n{}",
        failures.join("\n")
    );
}

fn filled_rectangles(shape: &Shape, rectangles: &mut Vec<(egui::Rect, Color32)>) {
    match shape {
        Shape::Rect(rect) => rectangles.push((rect.rect, rect.fill)),
        Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| filled_rectangles(shape, rectangles)),
        _ => {}
    }
}

#[test]
fn design_system_creator_notices_keep_soft_surfaces_and_action_order() {
    let notices = [
        (1, "Enter a valid maximum and inspect who can change it."),
        (
            2,
            "No one can enable or change recurring distribution later.",
        ),
        (
            3,
            "No groups yet. Use owner or identity rules, or add a group to share authority.",
        ),
        (
            4,
            "6 of 6 histories are selected. History settings are part of the reviewed contract.",
        ),
    ];
    let mut failures = Vec::new();
    for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
        with_isolated_data_dir(|| {
            let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
            harness.set_size(egui::vec2(1260.0, 900.0));
            apply_theme(&harness.ctx, mode);
            harness.run_steps(5);
            let mut current_section = 0;
            for (section, label) in notices {
                while current_section < section {
                    harness.get_by_label_contains("Continue:").click();
                    harness.run_steps(5);
                    current_section += 1;
                }
                harness
                    .query_all_by_label(label)
                    .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
                    .expect("The editor notice must exist")
                    .scroll_to_me();
                harness.run_steps(5);
                let text_rect = harness
                    .query_all_by_label(label)
                    .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
                    .expect("The editor notice must remain reachable")
                    .rect();
                let mut rectangles = Vec::new();
                let mut colors = Vec::new();
                for shape in &harness.output().shapes {
                    filled_rectangles(&shape.shape, &mut rectangles);
                    if shape.shape.visual_bounding_rect().intersects(text_rect) {
                        text_colors(&shape.shape, label, &mut colors);
                    }
                }
                let (expected_background, expected_foreground) = if section == 1 {
                    (
                        DashColors::message_background_color(
                            dash_evo_tool::ui::MessageType::Warning,
                            dark,
                        ),
                        DashColors::warning_color(dark),
                    )
                } else {
                    (DashColors::selected(dark), DashColors::text_primary(dark))
                };
                let background = rectangles.iter().find(|(rect, fill)| {
                    *fill == expected_background && rect.contains_rect(text_rect)
                });
                if background.is_none()
                    || colors.is_empty()
                    || colors.iter().any(|color| *color != expected_foreground)
                {
                    failures.push(format!(
                        "{mode:?}/{section}: notice needs the expected informational or warning surface: {label}"
                    ));
                }
                if section == 1 {
                    let amount = harness.get_by_role_and_label(
                        egui::accesskit::Role::TextInput,
                        "Maximum outstanding supply",
                    );
                    amount.scroll_to_me();
                    harness.run_steps(5);
                    harness
                        .get_by_role_and_label(
                            egui::accesskit::Role::TextInput,
                            "Maximum outstanding supply",
                        )
                        .focus();
                    harness.run_steps(2);
                    harness
                        .get_by_role_and_label(
                            egui::accesskit::Role::TextInput,
                            "Maximum outstanding supply",
                        )
                        .type_text("1000");
                    harness.run_steps(5);
                    let permanent = "The maximum is a permanent hard cap: no one can change it or change that rule.";
                    harness
                        .query_all_by_label(permanent)
                        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
                        .expect("Valid locked cap explanation")
                        .scroll_to_me();
                    harness.run_steps(5);
                    let rect = harness
                        .query_all_by_label(permanent)
                        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
                        .expect("Permanent cap notice")
                        .rect();
                    let mut fills = Vec::new();
                    for shape in &harness.output().shapes {
                        filled_rectangles(&shape.shape, &mut fills);
                    }
                    if !fills.iter().any(|(fill_rect, color)| {
                        *color == DashColors::selected(dark) && fill_rect.contains_rect(rect)
                    }) {
                        failures.push(format!(
                            "{mode:?}: valid permanent cap must use the informational surface"
                        ));
                    }
                }
                if section == 3
                    && text_rect.top() <= harness.get_by_label("Add group").rect().bottom()
                {
                    failures.push(format!(
                        "{mode:?}: empty-group notice must follow Add group"
                    ));
                }
            }
        });
    }
    assert!(
        failures.is_empty(),
        "Contextual notices:\n{}",
        failures.join("\n")
    );
}

#[test]
fn design_system_recurring_authority_warning_precedes_expanded_fields() {
    use egui::accesskit::Role;
    for (mode, dark) in [(ThemeMode::Light, false), (ThemeMode::Dark, true)] {
        with_isolated_data_dir(|| {
            let mut harness = mount_app(RootScreenType::RootScreenTokenCreator);
            harness.set_size(egui::vec2(1260.0, 900.0));
            apply_theme(&harness.ctx, mode);
            harness.run_steps(5);
            for _ in 0..2 {
                harness.get_by_label_contains("Continue:").click();
                harness.run_steps(5);
            }
            harness
                .get_by_label("Enable recurring distribution")
                .click();
            harness.run_steps(5);
            let edit = "Edit permission to change recurring distribution";
            harness.get_by_label(edit).scroll_to_me();
            harness.run_steps(5);
            harness.get_by_label(edit).click();
            harness.run_steps(5);
            harness
                .get_by_role_and_label(Role::ComboBox, "Who can perform this action")
                .scroll_to_me();
            harness.run_steps(5);
            harness
                .get_by_role_and_label(Role::ComboBox, "Who can perform this action")
                .click();
            harness.run_steps(5);
            harness
                .get_by_role_and_label(Role::Button, "Contract owner")
                .click();
            harness.run_steps(5);
            harness.get_by_label("Done").click();
            harness.run_steps(5);
            let label = "Contract owner can change recurring distribution.";
            harness
                .query_all_by_label(label)
                .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
                .expect("The chosen authority must be shown")
                .scroll_to_me();
            harness.run_steps(5);
            let text_rect = harness
                .query_all_by_label(label)
                .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
                .expect("The authority notice must remain reachable")
                .rect();
            let field = harness
                .get_by_role_and_label(Role::TextInput, "Repeat every")
                .rect();
            let mut rectangles = Vec::new();
            let mut colors = Vec::new();
            for shape in &harness.output().shapes {
                filled_rectangles(&shape.shape, &mut rectangles);
                if shape.shape.visual_bounding_rect().intersects(text_rect) {
                    text_colors(&shape.shape, label, &mut colors);
                }
            }
            let warning = rectangles.iter().any(|(rect, fill)| {
                *fill
                    == DashColors::message_background_color(
                        dash_evo_tool::ui::MessageType::Warning,
                        dark,
                    )
                    && rect.contains_rect(text_rect)
            });
            assert!(
                warning
                    && !colors.is_empty()
                    && colors
                        .iter()
                        .all(|color| *color == DashColors::warning_color(dark))
                    && text_rect.bottom() < field.top(),
                "{mode:?}: active authority must be a warning before the long form: warning={warning}, colors={colors:?}, notice={text_rect:?}, first field={field:?}"
            );
        });
    }
}
