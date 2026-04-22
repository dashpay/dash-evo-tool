use std::time::Duration;

use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::{BannerStatus, Component, ComponentResponse, MessageBanner};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Test that show_global renders nothing and does not panic when no message is set.
#[test]
fn test_banner_renders_nothing_when_empty() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(400.0, 200.0))
        .build_ui(|ui| {
            MessageBanner::show_global(ui);
        });
    harness.run();
    // No labels should be present (empty banner renders nothing)
    assert!(harness.query_by_label("\u{274C}").is_none());
}

/// Test the global set/has/clear cycle using a standalone egui::Context.
#[test]
fn test_global_set_and_has() {
    let ctx = egui::Context::default();

    // Initially no global message
    assert!(!MessageBanner::has_global(&ctx));

    // Set a global error message
    let handle = MessageBanner::set_global(&ctx, "Something went wrong", MessageType::Error);
    assert!(MessageBanner::has_global(&ctx));
    assert!(handle.elapsed().is_some());

    // Clear specific message
    MessageBanner::clear_global_message(&ctx, "Something went wrong");
    assert!(!MessageBanner::has_global(&ctx));

    // Handle should now report None for elapsed (banner gone)
    assert!(handle.elapsed().is_none());
}

/// Test the per-instance set_message / has_message / clear cycle.
#[test]
fn test_instance_set_and_has() {
    let mut banner = MessageBanner::new();

    assert!(!banner.has_message());
    assert_eq!(banner.current_value(), None);

    banner.set_message("Error occurred", MessageType::Error);
    assert!(banner.has_message());
    assert_eq!(banner.current_value(), Some(BannerStatus::Visible));

    banner.clear();
    assert!(!banner.has_message());
    assert_eq!(banner.current_value(), None);
}

/// Test that a global error message renders and the text appears.
#[test]
fn test_banner_renders_error_message() {
    let message_text = "Critical failure detected";

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(|ui| {
            MessageBanner::set_global(ui.ctx(), message_text, MessageType::Error);
            MessageBanner::show_global(ui);
        });
    harness.run();
    // The message text and dismiss button should be present
    assert!(harness.query_by_label(message_text).is_some());
    assert!(harness.query_by_label("\u{274C}").is_some());
    // Error banners auto-dismiss after 9s, so countdown should be present
    assert!(harness.query_by_label_contains("s)").is_some());
}

/// Test that all four MessageType variants render with correct text and icon.
#[test]
fn test_banner_renders_all_types() {
    let variants = [
        (MessageType::Error, "Error message", "\u{26D4}"),
        (MessageType::Warning, "Warning message", "\u{26A0}"),
        (MessageType::Success, "Success message", "\u{2705}"),
        (MessageType::Info, "Info message", "\u{1F4AC}"),
    ];

    for (msg_type, text, icon) in variants {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 200.0))
            .build_ui(move |ui| {
                MessageBanner::set_global(ui.ctx(), text, msg_type);
                MessageBanner::show_global(ui);
            });
        harness.run();
        // Message text, icon, and dismiss button should all be present
        assert!(
            harness.query_by_label(text).is_some(),
            "Missing text for {:?}",
            msg_type
        );
        assert!(
            harness.query_by_label(icon).is_some(),
            "Missing icon for {:?}",
            msg_type
        );
        assert!(
            harness.query_by_label("\u{274C}").is_some(),
            "Missing dismiss button for {:?}",
            msg_type
        );
    }
}

/// Test that multiple different messages coexist (not overwritten).
#[test]
fn test_multiple_messages_coexist() {
    let ctx = egui::Context::default();

    let handle1 = MessageBanner::set_global(&ctx, "Error one", MessageType::Error);
    let handle2 = MessageBanner::set_global(&ctx, "Error two", MessageType::Error);
    assert!(MessageBanner::has_global(&ctx));
    assert!(handle1.elapsed().is_some());
    assert!(handle2.elapsed().is_some());

    // Clear first — second should still exist
    MessageBanner::clear_global_message(&ctx, "Error one");
    assert!(MessageBanner::has_global(&ctx));
    assert!(handle1.elapsed().is_none());
    assert!(handle2.elapsed().is_some());

    // Clear second — nothing left
    MessageBanner::clear_global_message(&ctx, "Error two");
    assert!(!MessageBanner::has_global(&ctx));
    assert!(handle2.elapsed().is_none());
}

/// Test that duplicate text is deduplicated (idempotent set_global).
#[test]
fn test_deduplication() {
    let ctx = egui::Context::default();

    let handle1 = MessageBanner::set_global(&ctx, "Same message", MessageType::Error);
    let handle2 = MessageBanner::set_global(&ctx, "Same message", MessageType::Error);
    let handle3 = MessageBanner::set_global(&ctx, "Same message", MessageType::Error);

    // All handles should reference the same banner (all alive)
    assert!(handle1.elapsed().is_some());
    assert!(handle2.elapsed().is_some());
    assert!(handle3.elapsed().is_some());

    // Only one should exist — clear_global_message removes it, then nothing left
    MessageBanner::clear_global_message(&ctx, "Same message");
    assert!(!MessageBanner::has_global(&ctx));

    // All handles should now be dead
    assert!(handle1.elapsed().is_none());
    assert!(handle2.elapsed().is_none());
    assert!(handle3.elapsed().is_none());
}

/// Test replace_global finds and replaces an existing message.
#[test]
fn test_replace_global_finds_and_replaces() {
    let ctx = egui::Context::default();

    let original_handle = MessageBanner::set_global(&ctx, "Generic success", MessageType::Success);
    MessageBanner::set_global(&ctx, "An error", MessageType::Error);

    // Replace the generic one
    let replaced_handle = MessageBanner::replace_global(
        &ctx,
        "Generic success",
        "Specific success",
        MessageType::Success,
    );

    // Both handles should point to the same banner (same key, text changed)
    assert!(original_handle.elapsed().is_some());
    assert!(replaced_handle.elapsed().is_some());

    // The old text should be gone, new text should exist
    // Clearing "Generic success" should be a no-op (already replaced)
    MessageBanner::clear_global_message(&ctx, "Generic success");
    // "Specific success" and "An error" should still be present
    assert!(MessageBanner::has_global(&ctx));

    // Clear the remaining two
    MessageBanner::clear_global_message(&ctx, "Specific success");
    MessageBanner::clear_global_message(&ctx, "An error");
    assert!(!MessageBanner::has_global(&ctx));
}

/// Test replace_global with unknown old_text adds as new.
#[test]
fn test_replace_global_adds_when_not_found() {
    let ctx = egui::Context::default();

    let handle =
        MessageBanner::replace_global(&ctx, "nonexistent", "New message", MessageType::Info);
    assert!(MessageBanner::has_global(&ctx));
    assert!(handle.elapsed().is_some());

    MessageBanner::clear_global_message(&ctx, "New message");
    assert!(!MessageBanner::has_global(&ctx));
    assert!(handle.elapsed().is_none());
}

/// Test clear_global_message removes only the specific message.
#[test]
fn test_clear_global_message_removes_specific() {
    let ctx = egui::Context::default();

    let keep_handle = MessageBanner::set_global(&ctx, "Keep this", MessageType::Error);
    let remove_handle = MessageBanner::set_global(&ctx, "Remove this", MessageType::Warning);

    MessageBanner::clear_global_message(&ctx, "Remove this");
    assert!(MessageBanner::has_global(&ctx));
    assert!(keep_handle.elapsed().is_some());
    assert!(remove_handle.elapsed().is_none());

    MessageBanner::clear_global_message(&ctx, "Keep this");
    assert!(!MessageBanner::has_global(&ctx));
    assert!(keep_handle.elapsed().is_none());
}

/// Test that empty string is a no-op for set_global (does not clear).
#[test]
fn test_set_empty_string_is_noop() {
    let ctx = egui::Context::default();

    MessageBanner::set_global(&ctx, "Existing", MessageType::Info);
    let empty_handle = MessageBanner::set_global(&ctx, "", MessageType::Info);
    // Empty string should not clear existing messages
    assert!(MessageBanner::has_global(&ctx));
    // Empty handle should not reference any real banner
    assert!(empty_handle.elapsed().is_none());
}

/// Test that per-instance set_message with empty string clears.
#[test]
fn test_instance_set_empty_string_clears() {
    let mut banner = MessageBanner::new();
    banner.set_message("Some message", MessageType::Warning);
    assert!(banner.has_message());
    assert_eq!(banner.current_value(), Some(BannerStatus::Visible));

    banner.set_message("", MessageType::Warning);
    assert!(!banner.has_message());
    assert_eq!(banner.current_value(), None);
}

/// Test that a per-instance banner renders via Component::show() and returns Visible status.
#[test]
fn test_instance_banner_rendering() {
    let mut banner = MessageBanner::new();
    banner.set_message("Instance error", MessageType::Error);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            let response = banner.show(ui);
            assert_eq!(response.inner.status, Some(BannerStatus::Visible));
            assert!(!response.inner.has_changed());
            assert!(response.inner.is_valid());
            assert!(response.inner.error_message().is_none());
        });
    harness.run();
    assert!(harness.query_by_label("Instance error").is_some());
    assert!(harness.query_by_label("\u{274C}").is_some());
}

/// Test that Component::show() returns None status when no message is set.
#[test]
fn test_instance_banner_empty_status() {
    let mut banner = MessageBanner::new();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            let response = banner.show(ui);
            assert_eq!(response.inner.status, None);
            assert!(!response.inner.has_changed());
            assert!(response.inner.is_valid());
            assert!(response.inner.error_message().is_none());
        });
    harness.run();
    // Empty banner renders nothing
    assert!(harness.query_by_label("\u{274C}").is_none());
}

/// Test that current_value() returns Visible when message is set, None otherwise.
#[test]
fn test_instance_current_value() {
    let mut banner = MessageBanner::new();
    assert_eq!(banner.current_value(), None);

    banner.set_message("Some error", MessageType::Error);
    assert_eq!(banner.current_value(), Some(BannerStatus::Visible));

    banner.clear();
    assert_eq!(banner.current_value(), None);
}

/// Test that multiple banners render and all texts appear.
#[test]
fn test_multiple_banners_render() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui(|ui| {
            MessageBanner::set_global(ui.ctx(), "Error one", MessageType::Error);
            MessageBanner::set_global(ui.ctx(), "Warning one", MessageType::Warning);
            MessageBanner::set_global(ui.ctx(), "Success one", MessageType::Success);
            assert!(MessageBanner::has_global(ui.ctx()));
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(harness.query_by_label("Error one").is_some());
    assert!(harness.query_by_label("Warning one").is_some());
    assert!(harness.query_by_label("Success one").is_some());
    // Each banner has its own dismiss button
    assert_eq!(harness.get_all_by_label("\u{274C}").count(), 3);
}

/// Test BannerHandle::clear() removes the banner.
#[test]
fn test_handle_clear() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "To be cleared", MessageType::Info);
    assert!(MessageBanner::has_global(&ctx));

    handle.clear();
    assert!(!MessageBanner::has_global(&ctx));
}

/// Test BannerHandle::set_message() updates the banner text.
#[test]
fn test_handle_set_message() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Original text", MessageType::Info);
    assert!(handle.set_message("Updated text").is_some());

    // Old text should not match anymore
    MessageBanner::clear_global_message(&ctx, "Original text");
    // Banner should still exist under the new text
    assert!(MessageBanner::has_global(&ctx));
    assert!(handle.elapsed().is_some());

    // Clearing the new text should remove it
    MessageBanner::clear_global_message(&ctx, "Updated text");
    assert!(!MessageBanner::has_global(&ctx));
}

/// Test BannerHandle::set_message() returns None on a cleared banner.
#[test]
fn test_handle_set_message_on_cleared_banner() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Will be cleared", MessageType::Info);
    MessageBanner::clear_global_message(&ctx, "Will be cleared");

    assert!(handle.set_message("New text").is_none());
    // No banner should have been created
    assert!(!MessageBanner::has_global(&ctx));
}

/// Test BannerHandle::with_auto_dismiss() returns None on a cleared banner.
#[test]
fn test_handle_with_auto_dismiss_on_cleared_banner() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Temporary", MessageType::Info);
    MessageBanner::clear_global_message(&ctx, "Temporary");

    assert!(handle.with_auto_dismiss(Duration::from_secs(10)).is_none());
}

/// Test BannerHandle::with_elapsed() returns None on a cleared banner.
#[test]
fn test_handle_with_elapsed_on_cleared_banner() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Gone", MessageType::Info);
    MessageBanner::clear_global_message(&ctx, "Gone");

    assert!(handle.with_elapsed().is_none());
}

/// Test BannerHandle::with_elapsed() returns Some on a live banner.
#[test]
fn test_handle_with_elapsed_on_live_banner() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Loading...", MessageType::Info);
    assert!(handle.with_elapsed().is_some());
    assert!(handle.elapsed().is_some());
}

/// Test that handle.clear() only removes the specific banner, not others.
#[test]
fn test_handle_clear_leaves_other_banners() {
    let ctx = egui::Context::default();

    let handle1 = MessageBanner::set_global(&ctx, "Banner one", MessageType::Error);
    let handle2 = MessageBanner::set_global(&ctx, "Banner two", MessageType::Warning);

    handle1.clear();
    assert!(MessageBanner::has_global(&ctx));
    assert!(handle2.elapsed().is_some());

    handle2.clear();
    assert!(!MessageBanner::has_global(&ctx));
}

/// Test per-instance set_auto_dismiss builder chaining.
#[test]
fn test_instance_set_auto_dismiss() {
    let mut banner = MessageBanner::new();
    banner
        .set_message("Timed message", MessageType::Error)
        .set_auto_dismiss(Duration::from_secs(10));
    assert!(banner.has_message());
    assert_eq!(banner.current_value(), Some(BannerStatus::Visible));
}

/// Test that replace_global with empty new_text clears the old message.
#[test]
fn test_replace_global_empty_new_text_clears() {
    let ctx = egui::Context::default();

    MessageBanner::set_global(&ctx, "Old message", MessageType::Info);
    assert!(MessageBanner::has_global(&ctx));

    let handle = MessageBanner::replace_global(&ctx, "Old message", "", MessageType::Info);
    assert!(!MessageBanner::has_global(&ctx));
    // Handle for empty replacement should not reference a real banner
    assert!(handle.elapsed().is_none());
}

/// Test BannerHandle::with_auto_dismiss() returns Some on a live banner.
#[test]
fn test_handle_with_auto_dismiss_on_live_banner() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Dismissable", MessageType::Error);
    assert!(handle.with_auto_dismiss(Duration::from_secs(30)).is_some());
    assert!(handle.elapsed().is_some());
}

/// Test that exceeding MAX_BANNERS (5) evicts the oldest banner.
#[test]
fn test_max_banners_eviction() {
    let ctx = egui::Context::default();

    let handle1 = MessageBanner::set_global(&ctx, "Banner 1", MessageType::Error);
    let handle2 = MessageBanner::set_global(&ctx, "Banner 2", MessageType::Error);
    let _handle3 = MessageBanner::set_global(&ctx, "Banner 3", MessageType::Error);
    let _handle4 = MessageBanner::set_global(&ctx, "Banner 4", MessageType::Error);
    let _handle5 = MessageBanner::set_global(&ctx, "Banner 5", MessageType::Error);

    // All 5 should be alive
    assert!(handle1.elapsed().is_some());
    assert!(handle2.elapsed().is_some());

    // Adding a 6th should evict the oldest (Banner 1)
    let handle6 = MessageBanner::set_global(&ctx, "Banner 6", MessageType::Error);
    assert!(handle1.elapsed().is_none()); // evicted
    assert!(handle2.elapsed().is_some()); // still alive
    assert!(handle6.elapsed().is_some()); // newly added

    // Adding a 7th should evict Banner 2
    let _handle7 = MessageBanner::set_global(&ctx, "Banner 7", MessageType::Error);
    assert!(handle2.elapsed().is_none()); // evicted
}

/// Test that replace_global resets show_elapsed flag.
#[test]
fn test_replace_global_resets_show_elapsed() {
    let ctx = egui::Context::default();

    let handle = MessageBanner::set_global(&ctx, "Loading...", MessageType::Info);
    handle.with_elapsed();

    // Replace should reset show_elapsed and set fresh auto_dismiss
    let replaced = MessageBanner::replace_global(&ctx, "Loading...", "Done!", MessageType::Success);
    assert!(replaced.elapsed().is_some());

    // The replaced banner should have auto-dismiss (Success type default),
    // not the elapsed mode from the original
    // We verify by checking it's still alive (just created, within 5s window)
    assert!(replaced.elapsed().unwrap() < Duration::from_secs(1));
}
