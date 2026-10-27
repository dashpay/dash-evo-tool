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
//! - Primary button `Set up your social profile` present.
//! - No request cards or active contacts list rendered (the populated-state
//!   section headings and the search placeholder must be absent).

use dash_evo_tool::ui::identity::contacts;
use dash_evo_tool::ui::identity::social_profile_gate_card::{
    HEADING as GATE_HEADING, PRIMARY_LABEL as GATE_PRIMARY,
};
use egui::accesskit::Role;
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

    // Primary CTA present, with the reworded label.
    assert_eq!(GATE_PRIMARY, "Set up your social profile");
    assert!(
        harness.query_by_label(GATE_PRIMARY).is_some(),
        "gated Contacts tab must show the `{GATE_PRIMARY}` primary button"
    );

    // The dead `Why?` button was removed — it must not render.
    assert!(
        harness.query_by_label("Why?").is_none(),
        "the non-functional `Why?` button must be gone"
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

/// The selected identity can change while another screen is open, without the
/// hub resetting its contact lists. The tab must then show the new identity's
/// own lists, never the previous identity's.
#[test]
fn contacts_tab_follows_an_identity_selected_outside_the_hub() {
    use crate::identity_home::{seed_identity, seed_identity_with_id};
    use crate::support::{fresh_app_context, with_isolated_data_dir};
    use dash_evo_tool::backend_task::dashpay::ContactData;
    use dash_evo_tool::ui::identity::profile_cache::{ProfileCache, ProfileFields};
    use dash_evo_tool::ui::state::contacts_view::ContactsState;
    use dash_sdk::platform::Identifier;

    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let second = seed_identity_with_id(&app_context, 0xB6, vec![], vec![]);
        let first = seed_identity(&app_context, vec![], vec![]);
        let mut profiles = ProfileCache::default();
        for identity in [first, second] {
            profiles.record_saved(
                identity,
                ProfileFields {
                    display_name: "Alex".into(),
                    ..Default::default()
                },
            );
        }
        let heading = |count: usize| format!("{} · {count}", contacts::ACTIVE_HEADING_PREFIX);
        let render_context = app_context.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(960.0, 720.0))
            .build_ui_state(
                move |ui, state: &mut (ContactsState, ProfileCache)| {
                    let _ = contacts::render(
                        ui,
                        &render_context,
                        render_context.resolve_selected_identity().as_ref(),
                        &mut state.0,
                        &mut state.1,
                        &mut Default::default(),
                        &mut Vec::new(),
                    );
                },
                (ContactsState::default(), profiles),
            );
        // The loading spinner keeps repainting, so step instead of settling.
        harness.run_steps(2);
        harness.state_mut().0.record_contacts(vec![ContactData {
            identity_id: Identifier::from([0xD8; 32]),
            nickname: Some("Bao".into()),
            note: None,
            is_hidden: false,
            details_unreadable: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }]);
        harness.run();
        assert!(harness.query_by_label(&heading(1)).is_some());

        app_context.set_selected_identity(Some(second));
        harness.run_steps(2);
        assert!(
            harness.query_by_label(&heading(1)).is_none()
                && harness.query_by_label("Bao").is_none(),
            "one identity's contacts must never be listed under another"
        );
        assert!(
            harness.query_by_label(contacts::CONTACTS_LOADING).is_some(),
            "the new identity's own list is loading"
        );
        assert_eq!(harness.state().0.contacts_len(), 0);
    });
}

/// Until the first contact-list load lands, the Active contacts section says
/// it is loading. "You have no contacts yet." is a claim about the loaded
/// list, so it must not show before there is one.
#[test]
fn active_contacts_say_loading_until_the_first_load_lands() {
    use crate::identity_home::seed_identity;
    use crate::support::{fresh_app_context, with_isolated_data_dir};
    use dash_evo_tool::ui::identity::profile_cache::{ProfileCache, ProfileFields};
    use dash_evo_tool::ui::state::contacts_view::ContactsState;

    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        let identity = seed_identity(&app_context, vec![], vec![]);
        let mut profiles = ProfileCache::default();
        profiles.record_saved(
            identity,
            ProfileFields {
                display_name: "Alex".into(),
                ..Default::default()
            },
        );
        let render_context = app_context.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(960.0, 720.0))
            .build_ui_state(
                move |ui, state: &mut (ContactsState, ProfileCache)| {
                    let _ = contacts::render(
                        ui,
                        &render_context,
                        render_context.resolve_selected_identity().as_ref(),
                        &mut state.0,
                        &mut state.1,
                        &mut Default::default(),
                        &mut Vec::new(),
                    );
                },
                (ContactsState::default(), profiles),
            );
        // The loading spinner keeps repainting, so step instead of settling.
        harness.run_steps(2);
        assert!(
            harness.query_by_label(contacts::CONTACTS_LOADING).is_some(),
            "the section must say the contacts are loading"
        );
        assert!(
            harness.query_by_role(Role::ProgressIndicator).is_some(),
            "a spinner shows next to the loading text"
        );
        assert!(
            harness.query_by_label(contacts::NO_ACTIVE_EMPTY).is_none(),
            "an unloaded list must not be reported as empty"
        );
        assert!(
            harness
                .query_by_label(contacts::ACTIVE_HEADING_PREFIX)
                .is_some(),
            "the heading names the section without a count while loading"
        );
        assert!(
            harness.query_by_label_contains("· 0").is_none(),
            "an unloaded list must not be counted as zero contacts"
        );

        harness.state_mut().0.record_contacts(Vec::new());
        harness.run();
        assert!(harness.query_by_label(contacts::NO_ACTIVE_EMPTY).is_some());
        assert!(harness.query_by_label(contacts::CONTACTS_LOADING).is_none());
        assert!(harness.query_by_role(Role::ProgressIndicator).is_none());
    });
}

/// The tab's load is asked for separately from rendering. A gated tab has
/// nothing to load; a populated one loads its two lists once.
#[test]
fn contacts_tab_load_is_for_a_populated_tab_and_goes_out_once() {
    use crate::identity_home::seed_identity;
    use crate::support::{fresh_app_context, with_isolated_data_dir};
    use dash_evo_tool::app::AppAction;
    use dash_evo_tool::ui::identity::profile_cache::{ProfileCache, ProfileFields};
    use dash_evo_tool::ui::state::contacts_view::ContactsState;

    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        // The tab holds its proof-verifying reads until the masternode list
        // is synced; this test starts on a synced network.
        app_context.connection_status().set_masternodes_ready(true);
        let identity = seed_identity(&app_context, vec![], vec![]);
        let mut state = ContactsState::default();
        let mut profiles = ProfileCache::default();
        assert!(
            matches!(
                contacts::tab_load_action(&app_context, &mut state, &mut profiles),
                AppAction::None
            ),
            "a tab gated on a missing social profile must not load contacts"
        );

        profiles.record_saved(
            identity,
            ProfileFields {
                display_name: "Alex".into(),
                ..Default::default()
            },
        );
        app_context.connection_status().set_masternodes_ready(false);
        assert!(
            matches!(
                contacts::tab_load_action(&app_context, &mut state, &mut profiles),
                AppAction::None
            ),
            "the populated tab holds its reads until the masternode list is synced"
        );
        assert!(state.load_is_due(), "the held load is still owed");
        app_context.connection_status().set_masternodes_ready(true);
        let load = contacts::tab_load_action(&app_context, &mut state, &mut profiles);
        assert!(
            matches!(&load, AppAction::BackendTasksWithContexts(tasks, _) if tasks.len() == 2),
            "the populated tab loads its contacts and its requests, got {load:?}"
        );
        assert!(
            matches!(
                contacts::tab_load_action(&app_context, &mut state, &mut profiles),
                AppAction::None
            ),
            "and does not ask for them again"
        );
    });
}
