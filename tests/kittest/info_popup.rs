use dash_evo_tool::ui::components::info_popup::InfoPopup;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn test_renders_title_and_message() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Help", "This is helpful information.");
            popup.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Help").is_some());
    assert!(
        harness
            .query_by_label("This is helpful information.")
            .is_some()
    );
}

#[test]
fn test_renders_default_close_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Title", "Message");
            popup.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Close").is_some());
}

#[test]
fn test_custom_close_text() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Title", "Message").close_text("Dismiss");
            popup.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Dismiss").is_some());
    assert!(harness.query_by_label("Close").is_none());
}

#[test]
fn test_open_false_renders_nothing() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Title", "Message").open(false);
            popup.show(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Title").is_none());
    assert!(harness.query_by_label("Message").is_none());
    assert!(harness.query_by_label("Close").is_none());
}

#[test]
fn test_is_open_returns_correct_state() {
    let popup_open = InfoPopup::new("Title", "Message").open(true);
    assert!(popup_open.is_open());

    let popup_closed = InfoPopup::new("Title", "Message").open(false);
    assert!(!popup_closed.is_open());
}

#[test]
fn test_is_open_default_is_true() {
    let popup = InfoPopup::new("Title", "Message");
    assert!(popup.is_open());
}

#[test]
fn test_plain_text_paragraph_splits() {
    let message = "First paragraph.\n\nSecond paragraph.";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Info", message);
            popup.show(ui);
        });
    harness.run();
    // Each paragraph becomes a separate label
    assert!(harness.query_by_label("First paragraph.").is_some());
    assert!(harness.query_by_label("Second paragraph.").is_some());
}

#[test]
fn test_markdown_mode_renders() {
    let message = "**Bold text** and _italic_";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Markdown Info", message).markdown(true);
            popup.show(ui);
        });
    harness.run();
    // Title and close button should render
    assert!(harness.query_by_label("Markdown Info").is_some());
    assert!(harness.query_by_label("Close").is_some());
}

#[test]
fn test_show_returns_false_when_open_no_interaction() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Title", "Message");
            let response = popup.show(ui);
            // No interaction happened, popup should not be closed
            assert!(!response.inner);
        });
    harness.run();
}

#[test]
fn test_show_returns_false_when_not_open() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Title", "Message").open(false);
            let response = popup.show(ui);
            // Already closed, show returns false (was not freshly closed)
            assert!(!response.inner);
        });
    harness.run();
}

#[test]
fn test_builder_chaining() {
    let popup = InfoPopup::new("Title", "Message")
        .close_text("OK")
        .markdown(false)
        .open(true);
    assert!(popup.is_open());
}

#[test]
fn test_single_paragraph_no_split() {
    let message = "Just a single paragraph with no breaks.";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Info", message);
            popup.show(ui);
        });
    harness.run();
    assert!(
        harness
            .query_by_label("Just a single paragraph with no breaks.")
            .is_some()
    );
}

#[test]
fn test_single_newline_replaced_with_space() {
    let message = "Line one\nLine two";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            let mut popup = InfoPopup::new("Info", message);
            popup.show(ui);
        });
    harness.run();
    // Single newline is replaced with space within a paragraph
    assert!(harness.query_by_label("Line one Line two").is_some());
}
