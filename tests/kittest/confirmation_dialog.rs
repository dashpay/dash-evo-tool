use dash_evo_tool::ui::components::Component;
use dash_evo_tool::ui::components::component_trait::ComponentResponse;
use dash_evo_tool::ui::components::confirmation_dialog::{
    ConfirmationDialog, ConfirmationStatus, NOTHING,
};
use dash_evo_tool::ui::helpers::draw_modal_overlay;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn test_renders_title_and_message() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Delete Item", "Are you sure?");
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Delete Item").is_some());
    assert!(harness.query_by_label("Are you sure?").is_some());
}

#[test]
fn test_renders_default_confirm_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message");
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Confirm").is_some());
}

#[test]
fn test_renders_default_cancel_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message");
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Cancel").is_some());
}

#[test]
fn test_custom_confirm_text() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog =
                ConfirmationDialog::new("Title", "Message").confirm_text(Some("Yes, do it"));
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Yes, do it").is_some());
    assert!(harness.query_by_label("Confirm").is_none());
}

#[test]
fn test_custom_cancel_text() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message").cancel_text(Some("Nope"));
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Nope").is_some());
    assert!(harness.query_by_label("Cancel").is_none());
}

#[test]
fn test_confirm_text_nothing_hides_confirm_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message").confirm_text(NOTHING);
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Confirm").is_none());
    // Cancel should still be there
    assert!(harness.query_by_label("Cancel").is_some());
}

#[test]
fn test_cancel_text_nothing_hides_cancel_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message").cancel_text(NOTHING);
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Cancel").is_none());
    // Confirm should still be there
    assert!(harness.query_by_label("Confirm").is_some());
}

#[test]
fn test_both_buttons_hidden_with_nothing() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message")
                .confirm_text(NOTHING)
                .cancel_text(NOTHING);
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Confirm").is_none());
    assert!(harness.query_by_label("Cancel").is_none());
    // Title and message should still render
    assert!(harness.query_by_label("Title").is_some());
    assert!(harness.query_by_label("Message").is_some());
}

#[test]
fn test_danger_mode_renders_confirm_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Delete", "This is destructive")
                .confirm_text(Some("Delete Forever"))
                .danger_mode(true);
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Delete Forever").is_some());
    assert!(harness.query_by_label("This is destructive").is_some());
}

#[test]
fn test_open_false_renders_nothing() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message").open(false);
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Title").is_none());
    assert!(harness.query_by_label("Message").is_none());
    assert!(harness.query_by_label("Confirm").is_none());
    assert!(harness.query_by_label("Cancel").is_none());
}

#[test]
fn test_initial_state_not_changed() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message");
            let response = dialog.show(ui);
            assert!(!response.inner.has_changed());
            assert!(response.inner.changed_value().is_none());
            assert!(response.inner.is_valid());
            assert!(response.inner.error_message().is_none());
        });
    harness.run();
}

#[test]
fn test_current_value_open_is_none() {
    let dialog = ConfirmationDialog::new("Title", "Message");
    assert_eq!(dialog.current_value(), None);
}

#[test]
fn test_current_value_closed_is_canceled() {
    let dialog = ConfirmationDialog::new("Title", "Message").open(false);
    assert_eq!(dialog.current_value(), Some(ConfirmationStatus::Canceled));
}

#[test]
fn test_default_construction() {
    let dialog = ConfirmationDialog::new("My Title", "My Message");
    assert_eq!(dialog.current_value(), None); // open by default
}

#[test]
fn test_chained_builder_methods() {
    let dialog = ConfirmationDialog::new("Title", "Msg")
        .confirm_text(Some("Go"))
        .cancel_text(Some("Back"))
        .danger_mode(true)
        .open(true);
    // Should not panic, and dialog is open
    assert_eq!(dialog.current_value(), None);
}

#[test]
fn test_danger_mode_with_custom_texts() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Danger Zone", "Are you really sure?")
                .confirm_text(Some("Destroy"))
                .cancel_text(Some("Keep"))
                .danger_mode(true);
            dialog.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Danger Zone").is_some());
    assert!(harness.query_by_label("Are you really sure?").is_some());
    assert!(harness.query_by_label("Destroy").is_some());
    assert!(harness.query_by_label("Keep").is_some());
}

#[test]
fn test_open_false_component_response() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut dialog = ConfirmationDialog::new("Title", "Message").open(false);
            let response = dialog.show(ui);
            // When dialog is not open, it should not report changes
            assert!(!response.inner.has_changed());
            assert!(response.inner.changed_value().is_none());
        });
    harness.run();
}

/// Regression guard for PR #732: a blocking confirmation modal must consume
/// pointer events so they cannot reach background widgets.
#[test]
fn test_modal_overlay_blocks_click_through_to_background_widget() {
    // Place a clickable background widget at a known rect that sits entirely
    // outside the centered dialog window but well within the screen-sized
    // modal. If modal chrome correctly consumes pointer events, the background
    // button must never be activated.
    let background_rect =
        egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(120.0, 30.0));

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui_state(
            |ui, clicks: &mut u32| {
                let response = ui.put(background_rect, egui::Button::new("Background"));
                if response.clicked() {
                    *clicks += 1;
                }
                let mut dialog = ConfirmationDialog::new("Modal Title", "Modal Message");
                dialog.show(ui);
            },
            0u32,
        );

    // Compute layout / paint so modal input blocking is registered.
    harness.run();

    // Sanity check: the dialog window does not cover the background button rect
    // (otherwise the click would be blocked by the dialog itself, not the overlay).
    let dialog_node = harness
        .query_by_label("Modal Title")
        .expect("dialog should be rendered");
    assert!(
        !dialog_node.rect().intersects(background_rect),
        "test setup invalid: background widget rect overlaps the dialog window rect"
    );

    // Click on the background button. The position is over the background
    // widget and inside the modal overlay, but outside the dialog window.
    let click_pos = background_rect.center();
    harness.event(egui::Event::PointerMoved(click_pos));
    harness.event(egui::Event::PointerButton {
        pos: click_pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos: click_pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();

    assert_eq!(
        *harness.state(),
        0,
        "modal chrome must consume pointer events; background widget was activated through the modal"
    );
}

/// Regression guard for legacy manual `egui::Window` modals that use
/// `draw_modal_overlay` directly instead of `modal_chrome`.
#[test]
fn test_legacy_modal_overlay_blocks_background_and_preserves_window_input() {
    let background_rect =
        egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(120.0, 30.0));

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui_state(
            |ui, clicks: &mut (u32, u32)| {
                let background_response = ui.put(background_rect, egui::Button::new("Background"));
                if background_response.clicked() {
                    clicks.0 += 1;
                }

                draw_modal_overlay(ui.ctx(), "legacy_helper_test_overlay");

                egui::Window::new("Legacy Modal")
                    .collapsible(false)
                    .resizable(false)
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ui.ctx(), |ui| {
                        if ui.button("Modal Button").clicked() {
                            clicks.1 += 1;
                        }
                    });
            },
            (0u32, 0u32),
        );

    harness.run();

    let modal_node = harness
        .query_by_label("Legacy Modal")
        .expect("legacy modal should be rendered");
    assert!(
        !modal_node.rect().intersects(background_rect),
        "test setup invalid: background widget rect overlaps the legacy modal rect"
    );

    let background_click_pos = background_rect.center();
    harness.event(egui::Event::PointerMoved(background_click_pos));
    harness.event(egui::Event::PointerButton {
        pos: background_click_pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos: background_click_pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();

    assert_eq!(
        harness.state().0,
        0,
        "legacy modal overlay must consume pointer events before background widgets"
    );

    let modal_button_rect = harness
        .query_by_label("Modal Button")
        .expect("modal button should be rendered")
        .rect();
    let modal_click_pos = modal_button_rect.center();
    harness.event(egui::Event::PointerMoved(modal_click_pos));
    harness.event(egui::Event::PointerButton {
        pos: modal_click_pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos: modal_click_pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();

    assert_eq!(
        harness.state().1,
        1,
        "foreground legacy modal window must remain interactive above the overlay sink"
    );
}
