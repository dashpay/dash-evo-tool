//! kittest coverage for the T-MIG-02 migration UX seam.
//!
//! These tests drive the public `MessageBanner` surface against the
//! same i18n-ready copy the app loop emits, so a regression in the
//! step-label table or the action-button plumbing fails here without
//! needing a full `AppState` harness.

use dash_evo_tool::app::{MIGRATION_RETRY_ACTION_ID, migration_running_text};
use dash_evo_tool::context::migration_status::MigrationStep;
use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::MessageBanner;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// TC-MIG-001 — when the migration enters its first step the banner
/// surfaces an Info-typed banner with the "Checking your wallet data."
/// label per Diziet §2.2 D-1.
#[test]
fn tc_mig_001_running_banner_shows_step_label() {
    let label = migration_running_text(MigrationStep::Detecting);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), label, MessageType::Info);
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(
        harness.query_by_label(label).is_some(),
        "running banner must render the step label verbatim",
    );
    // Dismiss control present — required by §2.3 a11y.
    assert!(harness.query_by_label("\u{274C}").is_some());
}

/// TC-MIG-014 — every `MigrationStep` exposes a single complete
/// sentence (i18n-extraction rule). Guards against future variants
/// landing without label coverage.
#[test]
fn tc_mig_014_running_text_covers_every_step_with_sentence() {
    for step in [
        MigrationStep::Detecting,
        MigrationStep::AppData,
        MigrationStep::SingleKey,
        MigrationStep::Shielded,
        MigrationStep::WalletSeeds,
        MigrationStep::WalletMeta,
        MigrationStep::Finalize,
    ] {
        let text = migration_running_text(step);
        assert!(!text.is_empty(), "step {step:?} has empty banner text");
        assert!(
            text.ends_with('.'),
            "step {step:?} text `{text}` is not a complete sentence",
        );
    }
}

/// TC-MIG-003 — failure banner gets the Retry action button and an
/// Error message type. The dismiss icon remains present so users can
/// hide the banner without retrying.
#[test]
fn tc_mig_003_failed_banner_shows_retry_button() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 220.0))
        .build_ui(|ui| {
            let handle = MessageBanner::set_global(
                ui.ctx(),
                "Storage update could not complete. Your data is safe.",
                MessageType::Error,
            );
            handle.with_action("Retry now", MIGRATION_RETRY_ACTION_ID);
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(
        harness
            .query_by_label("Storage update could not complete. Your data is safe.")
            .is_some(),
        "error banner text must be visible",
    );
    assert!(
        harness.query_by_label("Retry now").is_some(),
        "retry action must surface as a clickable button",
    );
    // TC-MIG-003 (continued) — the spec forbids "contact support" copy on
    // the failure banner; users must be able to self-resolve via Retry.
    assert!(
        harness.query_by_label("contact support").is_none(),
        "failure banner must not redirect to 'contact support'",
    );
    assert!(
        harness.query_by_label("Contact support").is_none(),
        "failure banner must not redirect to 'Contact support'",
    );
}

/// TC-MIG-005 / TC-A11Y-007 — clicking the Retry button enqueues the
/// retry action id, and `take_action` drains it FIFO for the app loop
/// to dispatch.
#[test]
fn tc_mig_005_retry_click_enqueues_action() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 220.0))
        .build_ui(|ui| {
            let handle = MessageBanner::set_global(
                ui.ctx(),
                "Storage update could not complete. Your data is safe.",
                MessageType::Error,
            );
            handle.with_action("Retry now", MIGRATION_RETRY_ACTION_ID);
            MessageBanner::show_global(ui);
        });
    harness.run();
    // Sanity: nothing pending before the click.
    assert!(MessageBanner::take_action(&harness.ctx).is_none());

    harness.get_by_label("Retry now").click();
    harness.run();

    let action = MessageBanner::take_action(&harness.ctx);
    assert_eq!(action.as_deref(), Some(MIGRATION_RETRY_ACTION_ID));
    // Drained: subsequent calls return None.
    assert!(MessageBanner::take_action(&harness.ctx).is_none());
}

/// TC-A11Y-004 — error banners ship an icon **and** a textual label
/// (color is not the only failure indicator). The ⛔ icon + the
/// "Storage update could not complete." prefix both render.
#[test]
fn tc_a11y_004_failure_banner_uses_icon_and_text() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(|ui| {
            MessageBanner::set_global(
                ui.ctx(),
                "Storage update could not complete. Your data is safe.",
                MessageType::Error,
            );
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(
        harness.query_by_label("\u{26D4}").is_some(),
        "error icon must render alongside text",
    );
    assert!(
        harness
            .query_by_label("Storage update could not complete. Your data is safe.")
            .is_some(),
    );
}
