//! kittest coverage for the T-MIG-02 migration UX seam.
//!
//! These tests drive the public `MessageBanner` surface against the
//! same i18n-ready copy the app loop emits, so a regression in the
//! step-label table or the action-button plumbing fails here without
//! needing a full `AppState` harness.

use dash_evo_tool::app::{
    MIGRATION_IDENTITIES_ACK_ACTION_ID, MIGRATION_RETRY_ACTION_ID,
    MIGRATION_UNREADABLE_ACK_ACTION_ID, MIGRATION_VOTES_ACK_ACTION_ID,
    migration_failed_with_unreadable_identities_text, migration_running_text,
    migration_unreadable_identities_and_votes_text, migration_unreadable_identities_text,
    migration_unreadable_votes_text,
};
use dash_evo_tool::context::migration_status::MigrationStep;
use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::components::MessageBanner;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// TC-MIG-001 — when the migration enters its first step the banner
/// surfaces an Info-typed banner with the "The app is checking your wallet data."
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
        MigrationStep::Identities,
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

/// QA-101 — a migration that drained the wallets but could not read some legacy
/// scheduled votes surfaces a Warning banner naming the recovery action, and
/// offers NO "Retry now": the drain is done and a corrupt row decodes no better
/// on a second pass, so a retry button would be a dead end.
#[test]
fn unreadable_votes_banner_warns_without_a_retry_action() {
    let text = migration_unreadable_votes_text(2);
    assert!(
        text.ends_with('.'),
        "banner copy must be a complete sentence for i18n extraction: `{text}`",
    );

    let label = text.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            MessageBanner::set_global(ui.ctx(), label.clone(), MessageType::Warning);
            MessageBanner::show_global(ui);
        });
    harness.run();

    assert!(
        harness.query_by_label(text.as_str()).is_some(),
        "the warning banner must render the unreadable-votes copy verbatim",
    );
    assert!(
        harness.query_by_label("Retry now").is_none(),
        "a completed drain must not offer a retry the user cannot benefit from",
    );
}

/// Fix-8 — the warning carries an explicit acknowledgement, and clicking it
/// enqueues the ack action id. The app loop drains that id and clears the
/// durable warning record; until then the banner returns on every launch, so a
/// vote whose deadline still matters cannot lose its only notice to a stray
/// dismissal.
#[test]
fn unreadable_votes_banner_acknowledgement_enqueues_action() {
    let text = migration_unreadable_votes_text(2);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 220.0))
        .build_ui(move |ui| {
            let handle = MessageBanner::set_global(ui.ctx(), text.clone(), MessageType::Warning);
            handle.with_action("Got it", MIGRATION_VOTES_ACK_ACTION_ID);
            MessageBanner::show_global(ui);
        });
    harness.run();
    assert!(MessageBanner::take_action(&harness.ctx).is_none());

    harness.get_by_label("Got it").click();
    harness.run();

    assert_eq!(
        MessageBanner::take_action(&harness.ctx).as_deref(),
        Some(MIGRATION_VOTES_ACK_ACTION_ID),
    );
}

/// Every message that tells the user their identities did not come across must
/// also tell them WHERE to get them back. "Load these identities again" named no
/// screen and no control, which leaves the Everyday User hunting for a flow they
/// may never have opened — the repo's error-message rules require a concrete,
/// self-serviceable action. The remedy lives behind "Load Identity" on the
/// Identities screen, so all three variants name both, exactly as the vote copy
/// names the Scheduled Votes screen.
#[test]
fn every_unreadable_identity_message_names_where_to_load_them_again() {
    for text in [
        migration_unreadable_identities_text(2),
        migration_unreadable_identities_and_votes_text(2, 3),
        migration_failed_with_unreadable_identities_text(1),
    ] {
        assert!(
            text.contains("Identities screen"),
            "the copy must name the screen that recovers the keys: `{text}`",
        );
        assert!(
            text.contains("Load Identity"),
            "the copy must name the control the user has to press: `{text}`",
        );
        assert!(
            text.ends_with('.'),
            "banner copy must be a complete sentence for i18n extraction: `{text}`",
        );
    }
}

/// The unreadable-identity warning is acknowledgeable, exactly like its vote
/// sibling. It used to render as a sticky, action-less banner: the user was told
/// their keys had not come across and given no way to say "I understand" — so the
/// warning returned on every launch with no gesture that could ever retire it.
/// Supersedes the pre-fix "warns without a retry action" coverage: it re-asserts
/// the same "no Retry now" fact (the drain is done; a corrupt row decodes no
/// better) and adds the acknowledgement round-trip the fix introduced.
#[test]
fn unreadable_identities_banner_acknowledgement_enqueues_action() {
    let text = migration_unreadable_identities_text(2);
    assert!(
        text.ends_with('.'),
        "banner copy must be a complete sentence for i18n extraction: `{text}`",
    );

    let label = text.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 220.0))
        .build_ui(move |ui| {
            let handle = MessageBanner::set_global(ui.ctx(), label.clone(), MessageType::Warning);
            handle.with_action("Got it", MIGRATION_IDENTITIES_ACK_ACTION_ID);
            MessageBanner::show_global(ui);
        });
    harness.run();

    assert!(
        harness.query_by_label(text.as_str()).is_some(),
        "the warning banner must render the unreadable-identities copy verbatim",
    );
    assert!(
        harness.query_by_label("Retry now").is_none(),
        "a completed drain must not offer a retry the user cannot benefit from",
    );
    assert!(MessageBanner::take_action(&harness.ctx).is_none());

    harness.get_by_label("Got it").click();
    harness.run();

    assert_eq!(
        MessageBanner::take_action(&harness.ctx).as_deref(),
        Some(MIGRATION_IDENTITIES_ACK_ACTION_ID),
        "the acknowledgement must reach the app loop, or the warning can never be retired",
    );
}

/// The combined banner names both problems, so its single "Got it" must enqueue
/// the combined acknowledgement — the one that retires BOTH durable records.
/// Routing it to either single-signal ack would leave the other half to re-raise
/// on the next launch, as a notice the user has already read. Supersedes the
/// pre-fix coverage that routed this banner's "Got it" to the vote-only ack —
/// that routing no longer matches the combined-ack design this ships.
#[test]
fn combined_unreadable_banner_acknowledgement_enqueues_the_combined_action() {
    let text = migration_unreadable_identities_and_votes_text(2, 3);
    let label = text.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 260.0))
        .build_ui(move |ui| {
            let handle = MessageBanner::set_global(ui.ctx(), label.clone(), MessageType::Warning);
            handle.with_action("Got it", MIGRATION_UNREADABLE_ACK_ACTION_ID);
            MessageBanner::show_global(ui);
        });
    harness.run();

    harness.get_by_label("Got it").click();
    harness.run();

    assert_eq!(
        MessageBanner::take_action(&harness.ctx).as_deref(),
        Some(MIGRATION_UNREADABLE_ACK_ACTION_ID),
    );
}

/// The one identity outcome that IS a failure: unreadable identities alongside a
/// hard app-data failure. Unlike its Warning siblings this renders as an Error with
/// a working "Retry now" — the app-data half genuinely did not finish, and its
/// sentinel is unwritten, so the retry re-runs it.
#[test]
fn failed_with_unreadable_identities_banner_offers_a_working_retry() {
    let text = migration_failed_with_unreadable_identities_text(1);
    let label = text.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 240.0))
        .build_ui(move |ui| {
            let handle = MessageBanner::set_global(ui.ctx(), label.clone(), MessageType::Error);
            handle.with_action("Retry now", MIGRATION_RETRY_ACTION_ID);
            MessageBanner::show_global(ui);
        });
    harness.run();

    assert!(
        harness.query_by_label(text.as_str()).is_some(),
        "the error banner must render the combined-failure copy verbatim",
    );
    assert!(MessageBanner::take_action(&harness.ctx).is_none());

    harness.get_by_label("Retry now").click();
    harness.run();

    assert_eq!(
        MessageBanner::take_action(&harness.ctx).as_deref(),
        Some(MIGRATION_RETRY_ACTION_ID),
        "the app-data half is retryable, so its banner's retry must enqueue the action",
    );
}
