use dash_evo_tool::ui::components::Component;
use dash_evo_tool::ui::components::component_trait::ComponentResponse;
use dash_evo_tool::ui::components::confirmation_dialog::{
    ConfirmationDialog, ConfirmationStatus, NOTHING,
};
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
