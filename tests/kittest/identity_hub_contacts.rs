//! IT-CONTACTS-01 — Contacts tab gated when no social profile.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/03-test-case-spec.md`
//! section `### IT-CONTACTS-01`.
//!
//! The full hub only routes to the Contacts tab when the active-network
//! identity count is ≥ 1 — the kittest DB-less harness used elsewhere has
//! zero identities and would render Onboarding instead. Rather than spin up
//! a real `AppContext` with injected fixtures (out of scope for the shell-
//! only T9 drop), this test mounts the gated render path directly: the
//! single source of truth that the hub calls when the active identity has no
//! social profile.
//!
//! The assertions cover the test-spec expectations:
//! - Heading `Set up a social profile first.` present.
//! - Primary button `Add a display name` present.
//! - No request cards or active contacts list rendered (the populated-state
//!   section headings and the search placeholder must be absent).

use dash_evo_tool::ui::identity::contacts;
use dash_evo_tool::ui::identity::social_profile_gate_card::{
    HEADING as GATE_HEADING, PRIMARY_LABEL as GATE_PRIMARY,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn it_contacts_01_gated_renders_when_no_social_profile() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(960.0, 720.0))
        .build_ui(|ui| {
            let _ = contacts::render_gated(ui, Some("alex.dash"));
        });
    harness.run();

    // Heading present (per IT-CONTACTS-01).
    assert!(
        harness.query_by_label(GATE_HEADING).is_some(),
        "gated Contacts tab must show the `{GATE_HEADING}` heading"
    );

    // Primary CTA present.
    assert!(
        harness.query_by_label(GATE_PRIMARY).is_some(),
        "gated Contacts tab must show the `{GATE_PRIMARY}` primary button"
    );

    // Populated-state copy must NOT appear when gated.
    assert!(
        harness.query_by_label(contacts::RECEIVED_HEADING).is_none(),
        "gated Contacts tab must NOT render the received-requests section"
    );
    assert!(
        harness
            .query_by_label_contains(contacts::ACTIVE_HEADING_PREFIX)
            .is_none(),
        "gated Contacts tab must NOT render the active-contacts section"
    );
    assert!(
        harness.query_by_label(contacts::SENT_HEADING).is_none(),
        "gated Contacts tab must NOT render the sent-requests section"
    );
    assert!(
        harness
            .query_by_label(contacts::SEARCH_PLACEHOLDER)
            .is_none(),
        "gated Contacts tab must NOT render the search input placeholder"
    );
}

#[test]
fn it_contacts_01_gated_handles_absent_dpns_handle() {
    // A user that has never registered a DPNS name: the gated card must
    // still render without emitting a stray `@{handle}` placeholder.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(960.0, 720.0))
        .build_ui(|ui| {
            let _ = contacts::render_gated(ui, None);
        });
    harness.run();

    assert!(
        harness.query_by_label(GATE_HEADING).is_some(),
        "gated Contacts tab must show its heading even without a DPNS handle"
    );
    assert!(
        harness.query_by_label(GATE_PRIMARY).is_some(),
        "gated Contacts tab must show its primary CTA without a DPNS handle"
    );
    // The raw placeholder must never survive into a rendered label.
    assert!(
        harness.query_by_label_contains("{handle}").is_none(),
        "gated Contacts tab must never render the raw `{{handle}}` placeholder"
    );
}
